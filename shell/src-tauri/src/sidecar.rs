//! III-3 — the kernel contract on the user's machine: **detect · start · refuse**.
//!
//! This is the ONE module that reverses the shell's fs/process posture
//! (`lib.rs` said "no fs, no shell, no http, no process plugins"). What it
//! adds is exactly two edges — filesystem (the data root, the consent marker,
//! the auth file, the log) and process (`std::process::Command` for the
//! bundled sidecar). It adds **no HTTP client**: the probe is a raw
//! `std::net::TcpStream` speaking `GET /health`, and no Tauri plugin is
//! installed. The webview still holds the typed wire; this module never sends
//! a request on its behalf.
//!
//! The contract, in one place:
//!
//! 1. **Expected** is the sealed provenance's `x_api_version`
//!    (`Contents/Resources/binaries/SIDECAR_PROVENANCE.json`), never the
//!    shell's own version and never a literal. Missing/unparseable is a
//!    refusal (fail closed).
//! 2. **Observed** is the `x-api-version` response header of `GET /health`;
//!    the body's `version` must agree with it. A listener that ANSWERS as
//!    something other than a Brain kernel is `Foreign` and is never killed.
//!    A port that is held but SILENT is `Unanswered`: it refuses at
//!    preflight (a held port is never spawned over) and is waited for inside
//!    the bounded start, because a kernel that has bound the port and is
//!    still booting says nothing at all.
//! 3. **Attach, never double-start.** A matching kernel is adopted. A
//!    mismatch, a foreign listener or an inconsistent answer is refused
//!    without spawning. Only `NotRunning` reaches the start path.
//! 4. **Consent before storage.** `consent=false` returns `ConsentRefused`
//!    BEFORE `create_dir_all(data_root)`. The marker lives outside the data
//!    root: `<app_config_dir>/consent-v1.json` (0600); the data root is
//!    `<app_data_dir>/kernel` (0700).
//! 5. **Authentication is mandatory.** The shell's token is resolved through
//!    the existing keychain seam (keychain → the shell's own
//!    `BRAIN_SHELL_TOKEN`); absent, 32 bytes from the OS random device are
//!    hex-encoded and stored through the same seam. The kernel gets
//!    `AUTH_TOKEN_FILE` (0600) and `BRAIN_REQUIRE_AUTH=1`. A keychain failure
//!    refuses; the sidecar never runs unauthenticated. No error, log line or
//!    DTO ever carries the token.
//! 6. **Spawn is an allowlist.** `current_exe().parent()/brain-server`, no
//!    arguments, `env_clear()` + the seven documented variables. No `sh -c`,
//!    no user-controlled input, no inherited `BIND_PUBLIC`.
//! 7. **The wait is bounded in Rust** (10 s, 200 ms polls, 300 ms per probe),
//!    not in the webview, so the refusal is enforceable. A timeout kills the
//!    child this process spawned and names the path, the wait and the last
//!    error.
//! 8. **Only a child this process owns is killed.** `shutdown` takes the
//!    managed `Child`; an attached kernel that the shell did not spawn is
//!    left alone.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::token;

/// The one port the shell's kernel listens on, and the loopback bind it is
/// given explicitly (never inherited).
pub const PORT: u16 = 8765;
pub const BIND: &str = "127.0.0.1";

/// Bounded start: 10 s total, 200 ms polls, 300 ms per probe (connect + read).
pub const WAIT_MS: u64 = 10_000;
pub const POLL_MS: u64 = 200;
pub const PROBE_TIMEOUT_MS: u64 = 300;

/// A health response is small; 16 KiB is the hard read ceiling.
const MAX_RESPONSE_BYTES: usize = 16 * 1024;

/// The resource (relative to `Contents/Resources`) and the auth/log names.
pub const PROVENANCE_REL: &str = "binaries/SIDECAR_PROVENANCE.json";
pub const CONSENT_FILE: &str = "consent-v1.json";
pub const AUTH_FILE: &str = "auth-token";
pub const LOG_FILE: &str = "brain-server.log";
const SIDECAR_NAME: &str = "brain-server";

// ── the pure contract types ────────────────────────────────────────────────

/// The sealed manifest, reduced to the two versions the contract reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub kernel_version: String,
    pub x_api_version: String,
}

/// A Brain kernel's `/health` answer: both versions, never merged silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub api_version: String,
    pub body_version: String,
}

/// What a single probe found.
///
/// `NotRunning` is the only state that may start. `Unanswered` (the port is
/// held, but nothing came back inside the timeout) and `Foreign` (something
/// ANSWERED, and it was not a Brain kernel) are two different facts and are
/// kept apart: the plan defines `Foreign` by the shape of the ANSWER, and
/// silence is silence. At preflight both refuse — a held port is never
/// spawned over — but inside the bounded wait only a WRONG answer ends the
/// wait early. A kernel that has bound the port and is still booting says
/// nothing, and a child that died on boot says nothing either; calling both
/// "foreign" would refuse a healthy kernel and misreport a crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    NotRunning,
    Unanswered,
    Foreign,
    Inconsistent { header: String, body: String },
    Probe(Probe),
}

/// The version decision. `Display` names both versions on a mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Ready,
    Mismatch { expected: String, observed: String },
    Foreign,
    Inconsistent { header: String, body: String },
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => write!(f, "the kernel matches the sealed provenance"),
            Self::Mismatch { expected, observed } => write!(
                f,
                "kernel version mismatch: expected {expected} (sealed sidecar provenance), observed {observed}"
            ),
            Self::Foreign => write!(
                f,
                "the listener on {BIND}:{PORT} did not answer as a Brain kernel"
            ),
            Self::Inconsistent { header, body } => write!(
                f,
                "the kernel disagrees with itself: x-api-version {header}, health body {body}"
            ),
        }
    }
}

/// A refusal. Every variant names its path/value; none carries the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    MissingProvenance {
        path: String,
        reason: String,
    },
    MissingSidecar {
        path: String,
    },
    ConsentRefused,
    TokenUnavailable,
    InsecureAuthFile {
        path: String,
        mode: u32,
    },
    Filesystem {
        path: String,
        reason: String,
    },
    ForeignListener {
        port: u16,
    },
    InconsistentVersions {
        header: String,
        body: String,
    },
    VersionMismatch {
        expected: String,
        observed: String,
    },
    StartTimeout {
        waited_ms: u64,
        sidecar: String,
        data_root: String,
        last: String,
    },
    SpawnFailed {
        sidecar: String,
        reason: String,
    },
}

impl ContractError {
    /// Stable machine codes for the webview. The UI maps these to copy; the
    /// diagnostic fields ride beside them, never inside them.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::MissingProvenance { .. } => "missingProvenance",
            Self::MissingSidecar { .. } => "missingSidecar",
            Self::ConsentRefused => "consentRefused",
            Self::TokenUnavailable => "tokenUnavailable",
            Self::InsecureAuthFile { .. } => "insecureAuthFile",
            Self::Filesystem { .. } => "filesystem",
            Self::ForeignListener { .. } => "foreign",
            Self::InconsistentVersions { .. } => "inconsistent",
            Self::VersionMismatch { .. } => "mismatch",
            Self::StartTimeout { .. } => "startTimeout",
            Self::SpawnFailed { .. } => "spawnFailed",
        }
    }
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingProvenance { path, reason } => {
                write!(
                    f,
                    "kernel provenance missing or unreadable: {path} ({reason})"
                )
            }
            Self::MissingSidecar { path } => {
                write!(f, "bundled kernel binary is missing: {path}")
            }
            Self::ConsentRefused => write!(f, "storage consent was not granted"),
            Self::TokenUnavailable => {
                write!(
                    f,
                    "the login keychain could not supply or store a kernel token"
                )
            }
            Self::InsecureAuthFile { path, mode } => write!(
                f,
                "refusing an auth file readable beyond its owner: {path} is mode {mode:04o}"
            ),
            Self::Filesystem { path, reason } => {
                write!(f, "filesystem refusal at {path}: {reason}")
            }
            Self::ForeignListener { port } => write!(
                f,
                "the listener on {BIND}:{port} is not a Brain kernel; it was not stopped"
            ),
            Self::InconsistentVersions { header, body } => write!(
                f,
                "the kernel's x-api-version header {header} disagrees with its health body {body}"
            ),
            Self::VersionMismatch { expected, observed } => write!(
                f,
                "kernel version mismatch: expected {expected}, observed {observed}"
            ),
            Self::StartTimeout {
                waited_ms,
                sidecar,
                data_root,
                last,
            } => write!(
                f,
                "the kernel did not answer within {waited_ms} ms (binary {sidecar}, data root {data_root}); last error: {last}"
            ),
            Self::SpawnFailed { sidecar, reason } => {
                write!(f, "could not start the bundled kernel {sidecar}: {reason}")
            }
        }
    }
}

/// The pre-spawn decision. `Start` is reachable ONLY from `NotRunning`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preflight {
    Attach { version: String },
    Start,
    Refuse(ContractError),
}

/// The bounded wait's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitOutcome {
    Ready { version: String },
    Refused(ContractError),
    Timeout { waited_ms: u64, last: String },
}

/// The pure command description: no `Command` is built in this type, so a
/// test can assert the allowlist without spawning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// The managed child. `Default` is the idle state.
#[derive(Default)]
pub struct KernelState {
    pub child: Mutex<Option<Child>>,
}

/// What `shutdown` did. A kill error is returned, never swallowed; the wait
/// still runs so the child is reaped.
#[derive(Debug, Default)]
pub struct ShutdownReport {
    pub status: Option<ExitStatus>,
    pub kill_error: Option<String>,
}

/// The wire shape every gate command returns. `state` is the discriminant;
/// the optional fields are the refusal's diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub state: String,
    pub kind: Option<String>,
    pub expected: Option<String>,
    pub observed: Option<String>,
    pub sidecar: Option<String>,
    pub data_root: Option<String>,
    pub waited_ms: Option<u64>,
    pub last_error: Option<String>,
    pub consent_granted: bool,
    pub version: Option<String>,
    pub pid: Option<u32>,
}

impl StatusDto {
    fn blank(state: &str, paths: Option<&AppPaths>, consent_granted: bool) -> Self {
        Self {
            state: state.to_string(),
            kind: None,
            expected: None,
            observed: None,
            sidecar: paths.map(|p| p.sidecar.display().to_string()),
            data_root: paths.map(|p| p.data_root.display().to_string()),
            waited_ms: None,
            last_error: None,
            consent_granted,
            version: None,
            pid: None,
        }
    }

    /// A matching kernel was found (or the sidecar answered after a start).
    pub fn ready(version: &str, paths: &AppPaths, consent_granted: bool) -> Self {
        let mut dto = Self::blank("ready", Some(paths), consent_granted);
        dto.version = Some(version.to_string());
        dto.expected = Some(version.to_string());
        dto
    }

    pub fn started(version: &str, pid: u32, paths: &AppPaths, consent_granted: bool) -> Self {
        let mut dto = Self::blank("started", Some(paths), consent_granted);
        dto.version = Some(version.to_string());
        dto.expected = Some(version.to_string());
        dto.pid = Some(pid);
        dto
    }

    pub fn already_running(version: &str, paths: &AppPaths, consent_granted: bool) -> Self {
        let mut dto = Self::blank("alreadyRunning", Some(paths), consent_granted);
        dto.version = Some(version.to_string());
        dto.expected = Some(version.to_string());
        dto
    }

    pub fn not_running(expected: &str, paths: &AppPaths, consent_granted: bool) -> Self {
        let mut dto = Self::blank("notRunning", Some(paths), consent_granted);
        dto.expected = Some(expected.to_string());
        dto
    }

    pub fn refused(error: ContractError, paths: Option<&AppPaths>, consent_granted: bool) -> Self {
        let mut dto = Self::blank("refused", paths, consent_granted);
        dto.kind = Some(error.kind().to_string());
        match &error {
            ContractError::VersionMismatch { expected, observed } => {
                dto.expected = Some(expected.clone());
                dto.observed = Some(observed.clone());
            }
            ContractError::InconsistentVersions { header, body } => {
                dto.expected = Some(header.clone());
                dto.observed = Some(body.clone());
            }
            ContractError::MissingSidecar { path } => dto.sidecar = Some(path.clone()),
            ContractError::MissingProvenance { path, reason } => {
                dto.last_error = Some(format!("{path}: {reason}"));
            }
            ContractError::InsecureAuthFile { path, mode } => {
                dto.last_error = Some(format!("{path} is mode {mode:04o}"));
            }
            ContractError::Filesystem { path, reason } => {
                dto.last_error = Some(format!("{path}: {reason}"));
            }
            ContractError::SpawnFailed { sidecar, reason } => {
                dto.sidecar = Some(sidecar.clone());
                dto.last_error = Some(reason.clone());
            }
            ContractError::StartTimeout {
                waited_ms,
                sidecar,
                data_root,
                last,
            } => {
                dto.waited_ms = Some(*waited_ms);
                dto.sidecar = Some(sidecar.clone());
                dto.data_root = Some(data_root.clone());
                dto.last_error = Some(last.clone());
            }
            ContractError::ConsentRefused
            | ContractError::TokenUnavailable
            | ContractError::ForeignListener { .. } => {}
        }
        dto
    }
}

// ── manifests and the probe ────────────────────────────────────────────────

/// `x.y.z` with optional `-prerelease`/`+build`; the kernel's version shape.
fn is_semver(value: &str) -> bool {
    let (core, rest) = match value.split_once('-') {
        Some((core, rest)) => (core, Some(rest)),
        None => (value, None),
    };
    let core = core.split_once('+').map_or(core, |(core, _)| core);
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    if !parts
        .iter()
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    {
        return false;
    }
    let Some(prerelease) = rest else {
        return true;
    };
    let prerelease = prerelease
        .split_once('+')
        .map_or(prerelease, |(pre, _)| pre);
    !prerelease.is_empty()
        && prerelease.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// Read the sealed manifest. Every failure is a refusal, never "unknown".
pub fn load_provenance(path: &Path) -> Result<Provenance, ContractError> {
    let bytes = fs::read(path).map_err(|error| ContractError::MissingProvenance {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| ContractError::MissingProvenance {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;

    let kernel_version = value
        .get("kernel_version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let x_api_version = value
        .get("x_api_version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let invalid = |reason: String| ContractError::MissingProvenance {
        path: path.display().to_string(),
        reason,
    };
    if kernel_version.is_empty() {
        return Err(invalid("kernel_version is absent or empty".to_string()));
    }
    if x_api_version.is_empty() {
        return Err(invalid("x_api_version is absent or empty".to_string()));
    }
    if !is_semver(&kernel_version) {
        return Err(invalid(format!(
            "kernel_version {kernel_version} is not a semver version"
        )));
    }
    if !is_semver(&x_api_version) {
        return Err(invalid(format!(
            "x_api_version {x_api_version} is not a semver version"
        )));
    }
    // NOT a rule: `kernel_version` and `x_api_version` are sealed from TWO
    // sources (the kernel's Cargo version and the wire's openapi version) and
    // the sealer checks each against its own source, not against the other.
    // A skew between them is a VERSION MISMATCH, and the contract reports
    // that — naming both versions on screen. Calling it a missing record
    // would be false (the file is present, readable and well formed) and it
    // would replace a named mismatch with an opaque sentence.
    Ok(Provenance {
        kernel_version,
        x_api_version,
    })
}

/// One `GET /health` over a raw stream. This function is the whole transport:
/// no client library, no Tauri capability, no second port.
pub fn probe(port: u16, timeout: Duration) -> ProbeOutcome {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, timeout) else {
        return ProbeOutcome::NotRunning;
    };
    if stream.set_read_timeout(Some(timeout)).is_err()
        || stream.set_write_timeout(Some(timeout)).is_err()
    {
        return ProbeOutcome::Foreign;
    }

    let request =
        format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        // The connect succeeded, so something holds the port; it just did
        // not speak HTTP. That is SILENCE, not a wrong answer — a child that
        // died on boot looks exactly like this.
        return ProbeOutcome::Unanswered;
    }

    let mut raw = Vec::new();
    if (&mut stream)
        .take(MAX_RESPONSE_BYTES as u64)
        .read_to_end(&mut raw)
        .is_err()
        && raw.is_empty()
    {
        // The peer held the connection and said nothing before the timeout.
        return ProbeOutcome::Unanswered;
    }
    if raw.is_empty() {
        return ProbeOutcome::Unanswered;
    }
    parse_health(&String::from_utf8_lossy(&raw))
}

fn parse_health(raw: &str) -> ProbeOutcome {
    let Some((head, body)) = raw.split_once("\r\n\r\n") else {
        // No header terminator: the answer never arrived. Not a wrong answer.
        return ProbeOutcome::Unanswered;
    };
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or("");
    if status.split_whitespace().nth(1) != Some("200") {
        return ProbeOutcome::Foreign;
    }

    let mut header = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("x-api-version")
        {
            header = Some(value.trim().to_string());
        }
    }
    let Some(header) = header else {
        return ProbeOutcome::Foreign;
    };
    if header.is_empty() {
        return ProbeOutcome::Foreign;
    }

    let Ok(json) = serde_json::from_str::<serde_json::Value>(body.trim()) else {
        return ProbeOutcome::Foreign;
    };
    if json.get("status").and_then(|v| v.as_str()) != Some("ok") {
        return ProbeOutcome::Foreign;
    }
    let Some(body_version) = json.get("version").and_then(|v| v.as_str()) else {
        return ProbeOutcome::Foreign;
    };
    let body_version = body_version.trim().to_string();
    if body_version.is_empty() {
        return ProbeOutcome::Foreign;
    }
    if header != body_version {
        return ProbeOutcome::Inconsistent {
            header,
            body: body_version,
        };
    }
    ProbeOutcome::Probe(Probe {
        api_version: header,
        body_version,
    })
}

/// The pure version decision.
pub fn decide(expected: &str, probe: &Probe) -> Decision {
    if probe.api_version != probe.body_version {
        return Decision::Inconsistent {
            header: probe.api_version.clone(),
            body: probe.body_version.clone(),
        };
    }
    if probe.api_version != expected {
        return Decision::Mismatch {
            expected: expected.to_string(),
            observed: probe.api_version.clone(),
        };
    }
    Decision::Ready
}

/// Detect before start. `Start` exists only on `NotRunning` + a present binary.
pub fn preflight(expected: &str, outcome: ProbeOutcome, sidecar: &Path) -> Preflight {
    match outcome {
        ProbeOutcome::Probe(probe) => match decide(expected, &probe) {
            Decision::Ready => Preflight::Attach {
                version: probe.api_version,
            },
            Decision::Mismatch { expected, observed } => {
                Preflight::Refuse(ContractError::VersionMismatch { expected, observed })
            }
            Decision::Foreign => Preflight::Refuse(ContractError::ForeignListener { port: PORT }),
            Decision::Inconsistent { header, body } => {
                Preflight::Refuse(ContractError::InconsistentVersions { header, body })
            }
        },
        ProbeOutcome::Foreign => Preflight::Refuse(ContractError::ForeignListener { port: PORT }),
        ProbeOutcome::Unanswered => {
            // A held port is never spawned over, whatever is holding it. The
            // refusal is the same shape as a foreign listener's: from outside
            // the contract, a port that does not answer as a Brain kernel is
            // not ours to touch.
            Preflight::Refuse(ContractError::ForeignListener { port: PORT })
        }
        ProbeOutcome::Inconsistent { header, body } => {
            Preflight::Refuse(ContractError::InconsistentVersions { header, body })
        }
        ProbeOutcome::NotRunning => {
            if sidecar.is_file() {
                Preflight::Start
            } else {
                Preflight::Refuse(ContractError::MissingSidecar {
                    path: sidecar.display().to_string(),
                })
            }
        }
    }
}

// ── consent, auth and the command allowlist ────────────────────────────────

/// The consent gate is the FIRST thing that may create the data root. With
/// `granted == false` it returns before any `create_dir_all`, so Cancel leaves
/// no directory to clean up. The marker lives in `config_dir`, outside the
/// data root, and is itself 0600.
pub fn consent_gate(
    config_dir: &Path,
    data_root: &Path,
    granted: bool,
) -> Result<PathBuf, ContractError> {
    if !granted {
        return Err(ContractError::ConsentRefused);
    }
    fs::create_dir_all(config_dir).map_err(|error| ContractError::Filesystem {
        path: config_dir.display().to_string(),
        reason: error.to_string(),
    })?;
    fs::set_permissions(config_dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
        ContractError::Filesystem {
            path: config_dir.display().to_string(),
            reason: error.to_string(),
        }
    })?;

    let marker = config_dir.join(CONSENT_FILE);
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&marker)
        .map_err(|error| ContractError::Filesystem {
            path: marker.display().to_string(),
            reason: error.to_string(),
        })?;
    fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).map_err(|error| {
        ContractError::Filesystem {
            path: marker.display().to_string(),
            reason: error.to_string(),
        }
    })?;
    file.write_all(b"{\"granted\":true,\"version\":1}\n")
        .and_then(|()| file.sync_all())
        .map_err(|error| ContractError::Filesystem {
            path: marker.display().to_string(),
            reason: error.to_string(),
        })?;

    fs::create_dir_all(data_root).map_err(|error| ContractError::Filesystem {
        path: data_root.display().to_string(),
        reason: error.to_string(),
    })?;
    fs::set_permissions(data_root, fs::Permissions::from_mode(0o700)).map_err(|error| {
        ContractError::Filesystem {
            path: data_root.display().to_string(),
            reason: error.to_string(),
        }
    })?;
    Ok(marker)
}

/// The marker is the "consent already given" witness; its absence is the
/// first-run state. A marker is never read for content — presence is consent.
pub fn consent_granted(config_dir: &Path) -> bool {
    config_dir.join(CONSENT_FILE).is_file()
}

/// Write `<data_root>/auth-token` at 0600, mirroring the kernel's
/// `read_owner_only_token` law BEFORE and AFTER the write. A pre-existing
/// group/world-readable file is refused, not silently chmodded: the kernel
/// would refuse to read it, and starting a sidecar that cannot authenticate
/// is the failure this check exists to prevent.
pub fn prepare_auth_file(data_root: &Path, token: &str) -> Result<PathBuf, ContractError> {
    fs::create_dir_all(data_root).map_err(|error| ContractError::Filesystem {
        path: data_root.display().to_string(),
        reason: error.to_string(),
    })?;
    let path = data_root.join(AUTH_FILE);

    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                return Err(ContractError::Filesystem {
                    path: path.display().to_string(),
                    reason: "auth-token is not a regular file".to_string(),
                });
            }
            let mode = metadata.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(ContractError::InsecureAuthFile {
                    path: path.display().to_string(),
                    mode,
                });
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ContractError::Filesystem {
                path: path.display().to_string(),
                reason: error.to_string(),
            });
        }
    }

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| ContractError::Filesystem {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
    file.write_all(format!("{token}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| ContractError::Filesystem {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;

    let mode = fs::metadata(&path)
        .map_err(|error| ContractError::Filesystem {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        return Err(ContractError::InsecureAuthFile {
            path: path.display().to_string(),
            mode,
        });
    }
    Ok(path)
}

/// 32 bytes from the OS random device, lowercase hex (64 chars).
pub fn generate_token() -> Result<String, ContractError> {
    let mut bytes = [0u8; 32];
    let mut source = File::open("/dev/urandom").map_err(|_| ContractError::TokenUnavailable)?;
    source
        .read_exact(&mut bytes)
        .map_err(|_| ContractError::TokenUnavailable)?;
    Ok(hex_32(&bytes))
}

/// The pure encoder — tests never touch `/dev/urandom`.
pub fn hex_32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// The command allowlist. Args are empty by construction; the environment is
/// exactly seven keys, and `BIND_HOST` is the loopback literal.
pub fn command_spec(sidecar: &Path, data_root: &Path, auth_file: &Path) -> SidecarCommand {
    SidecarCommand {
        program: sidecar.display().to_string(),
        args: Vec::new(),
        env: vec![
            (
                "AUTH_TOKEN_FILE".to_string(),
                auth_file.display().to_string(),
            ),
            ("BIND_HOST".to_string(), BIND.to_string()),
            ("BIND_PORT".to_string(), PORT.to_string()),
            (
                "BRAIN_DATA_ROOT".to_string(),
                data_root.display().to_string(),
            ),
            (
                "BRAIN_DB_PATH".to_string(),
                data_root.join("brain.db").display().to_string(),
            ),
            ("BRAIN_REQUIRE_AUTH".to_string(), "1".to_string()),
            ("RUST_LOG".to_string(), "warn".to_string()),
        ],
    }
}

/// Build the process from the allowlist: `env_clear` first, then exactly the
/// spec's variables. No `sh`, no `-c`, no inherited `BIND_PUBLIC`.
pub fn build_command(spec: &SidecarCommand) -> Command {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args).env_clear();
    for (name, value) in &spec.env {
        command.env(name, value);
    }
    command
}

/// The parent of the running executable + `brain-server`. In the bundle this
/// is `Contents/MacOS/brain-server`; the pin feeds the fake bundle path.
pub fn resolve_sidecar_path(exe: &Path) -> PathBuf {
    exe.parent()
        .unwrap_or_else(|| Path::new(""))
        .join(SIDECAR_NAME)
}

// ── the bounded wait and lifecycle ─────────────────────────────────────────

/// Poll the probe until the expected kernel answers or the deadline passes.
/// The deadline is checked BEFORE each sleep, so `wait_ms` shorter than the
/// poll interval still returns in bounded time. A refusal during the wait is
/// returned immediately (a stale sidecar may boot and answer wrongly).
pub fn wait_bounded<F>(mut probe_once: F, expected: &str, wait_ms: u64, poll_ms: u64) -> WaitOutcome
where
    F: FnMut() -> ProbeOutcome,
{
    let started = Instant::now();
    let deadline = started + Duration::from_millis(wait_ms);
    let mut last = format!("no response on {BIND}:{PORT}");
    loop {
        match probe_once() {
            ProbeOutcome::Probe(probe) => match decide(expected, &probe) {
                Decision::Ready => {
                    return WaitOutcome::Ready {
                        version: probe.api_version,
                    };
                }
                Decision::Mismatch { expected, observed } => {
                    return WaitOutcome::Refused(ContractError::VersionMismatch {
                        expected,
                        observed,
                    });
                }
                Decision::Inconsistent { header, body } => {
                    return WaitOutcome::Refused(ContractError::InconsistentVersions {
                        header,
                        body,
                    });
                }
                Decision::Foreign => {
                    return WaitOutcome::Refused(ContractError::ForeignListener { port: PORT });
                }
            },
            ProbeOutcome::Foreign => {
                return WaitOutcome::Refused(ContractError::ForeignListener { port: PORT });
            }
            ProbeOutcome::Unanswered => {
                // SILENCE is not a verdict: the child we spawned may still be
                // booting (it binds the port before it can answer a cold
                // start), and a child that died on boot says nothing either.
                // Both keep the wait alive until the deadline, which is what
                // turns a crash into a named StartTimeout instead of a false
                // "that is not a Brain kernel".
                last = format!("the port {BIND}:{PORT} is held but nothing answered");
            }
            ProbeOutcome::Inconsistent { header, body } => {
                return WaitOutcome::Refused(ContractError::InconsistentVersions { header, body });
            }
            ProbeOutcome::NotRunning => {
                last = format!("no response on {BIND}:{PORT}");
            }
        }

        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline - now;
        std::thread::sleep(remaining.min(Duration::from_millis(poll_ms)));
    }
    WaitOutcome::Timeout {
        waited_ms: started.elapsed().as_millis() as u64,
        last,
    }
}

/// Kill and reap the child this process holds. Idempotent: a second call does
/// nothing. An attached kernel (no held child) is never touched.
pub fn shutdown(state: &KernelState) -> ShutdownReport {
    let mut guard = state
        .child
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(mut child) = guard.take() else {
        return ShutdownReport::default();
    };
    let kill_error = child.kill().err().map(|error| error.to_string());
    let status = child.wait().ok();
    ShutdownReport { status, kill_error }
}

/// The `RunEvent::Exit` edge. Only the child this process spawned is held in
/// the managed state; an attached kernel has no entry and is never touched.
/// A kill failure is returned so the caller can say it out loud.
pub fn shutdown_app(app: &AppHandle) -> Option<String> {
    let state = app.state::<KernelState>();
    shutdown(state.inner()).kill_error
}

// ── the Tauri edges ────────────────────────────────────────────────────────

/// Every path the contract needs, resolved once.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub resource_dir: PathBuf,
    pub data_root: PathBuf,
    pub config_dir: PathBuf,
    pub sidecar: PathBuf,
}

fn app_paths(app: &AppHandle) -> Result<AppPaths, ContractError> {
    let resource_dir =
        app.path()
            .resource_dir()
            .map_err(|error| ContractError::MissingProvenance {
                path: PROVENANCE_REL.to_string(),
                reason: error.to_string(),
            })?;
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| ContractError::Filesystem {
            path: "app_data_dir".to_string(),
            reason: error.to_string(),
        })?;
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| ContractError::Filesystem {
            path: "app_config_dir".to_string(),
            reason: error.to_string(),
        })?;
    let executable = std::env::current_exe().map_err(|error| ContractError::Filesystem {
        path: "current_exe".to_string(),
        reason: error.to_string(),
    })?;
    Ok(AppPaths {
        resource_dir,
        data_root: data_dir.join("kernel"),
        config_dir,
        sidecar: resolve_sidecar_path(&executable),
    })
}

/// `kernel_status`: load the expected version, probe, decide. No spawn, no
/// directory creation.
pub fn status(app: &AppHandle) -> StatusDto {
    let paths = match app_paths(app) {
        Ok(paths) => paths,
        Err(error) => return StatusDto::refused(error, None, false),
    };
    let consent = consent_granted(&paths.config_dir);
    let manifest = paths.resource_dir.join(PROVENANCE_REL);
    let provenance = match load_provenance(&manifest) {
        Ok(provenance) => provenance,
        Err(error) => return StatusDto::refused(error, Some(&paths), consent),
    };
    let outcome = probe(PORT, Duration::from_millis(PROBE_TIMEOUT_MS));
    match preflight(&provenance.x_api_version, outcome, &paths.sidecar) {
        Preflight::Attach { version } => StatusDto::ready(&version, &paths, consent),
        Preflight::Start => StatusDto::not_running(&provenance.x_api_version, &paths, consent),
        Preflight::Refuse(error) => StatusDto::refused(error, Some(&paths), consent),
    }
}

/// `kernel_start`: consent → token → auth file → spawn → bounded wait. The
/// only creator of the data root, and the only caller of `Command::spawn`.
pub fn start(app: &AppHandle, state: &KernelState, consent: bool, wait_ms: u64) -> StatusDto {
    let paths = match app_paths(app) {
        Ok(paths) => paths,
        Err(error) => return StatusDto::refused(error, None, false),
    };
    start_with_paths(&paths, state, consent, wait_ms)
}

/// The start path with the paths already resolved. The Tauri edge above is
/// the only part that knows about `AppHandle`; this is the half a pin can
/// drive with temp directories — and the half where the ORDER of the
/// refusals is observable: a refusal creates no data root and spawns no
/// child, because the manifest is read before anything is written.
pub fn start_with_paths(
    paths: &AppPaths,
    state: &KernelState,
    consent: bool,
    wait_ms: u64,
) -> StatusDto {
    let manifest = paths.resource_dir.join(PROVENANCE_REL);
    let provenance = match load_provenance(&manifest) {
        Ok(provenance) => provenance,
        Err(error) => {
            return StatusDto::refused(error, Some(paths), consent_granted(&paths.config_dir));
        }
    };
    let consent = consent_granted(&paths.config_dir) || consent;

    let outcome = probe(PORT, Duration::from_millis(PROBE_TIMEOUT_MS));
    match preflight(&provenance.x_api_version, outcome, &paths.sidecar) {
        Preflight::Attach { version } => {
            return StatusDto::already_running(&version, paths, consent_granted(&paths.config_dir));
        }
        Preflight::Refuse(error) => {
            return StatusDto::refused(error, Some(paths), consent_granted(&paths.config_dir));
        }
        Preflight::Start => {}
    }

    // A held child that is still alive means a previous start is still
    // booting: wait for it rather than spawning a second process.
    let held_pid = {
        let mut guard = state
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => guard.as_ref().map(Child::id),
            Some(_) => {
                *guard = None;
                None
            }
            None => None,
        }
    };

    let pid = if let Some(pid) = held_pid {
        pid
    } else {
        if let Err(error) = consent_gate(&paths.config_dir, &paths.data_root, consent) {
            return StatusDto::refused(error, Some(paths), consent_granted(&paths.config_dir));
        }
        let token = match resolve_token() {
            Ok(token) => token,
            Err(error) => {
                return StatusDto::refused(error, Some(paths), consent_granted(&paths.config_dir));
            }
        };
        let auth_file = match prepare_auth_file(&paths.data_root, &token) {
            Ok(path) => path,
            Err(error) => {
                return StatusDto::refused(error, Some(paths), consent_granted(&paths.config_dir));
            }
        };
        let spec = command_spec(&paths.sidecar, &paths.data_root, &auth_file);
        let log = match open_log(&paths.data_root) {
            Ok(log) => log,
            Err(error) => {
                return StatusDto::refused(error, Some(paths), consent_granted(&paths.config_dir));
            }
        };
        let mut command = build_command(&spec);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log));
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return StatusDto::refused(
                    ContractError::SpawnFailed {
                        sidecar: spec.program,
                        reason: error.to_string(),
                    },
                    Some(paths),
                    consent_granted(&paths.config_dir),
                );
            }
        };
        let pid = child.id();
        let mut guard = state
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(child);
        pid
    };

    let outcome = wait_bounded(
        || probe(PORT, Duration::from_millis(PROBE_TIMEOUT_MS)),
        &provenance.x_api_version,
        wait_ms,
        POLL_MS,
    );
    match outcome {
        WaitOutcome::Ready { version } => {
            StatusDto::started(&version, pid, paths, consent_granted(&paths.config_dir))
        }
        WaitOutcome::Refused(error) => {
            let report = shutdown(state);
            let mut dto =
                StatusDto::refused(error, Some(paths), consent_granted(&paths.config_dir));
            append_kill_error(&mut dto, &report);
            dto
        }
        WaitOutcome::Timeout { waited_ms, last } => {
            let exited = child_exit(state);
            let last = match exited {
                Some(exit) => format!("{last}; the sidecar exited with {exit}"),
                None => last,
            };
            let error = ContractError::StartTimeout {
                waited_ms,
                sidecar: paths.sidecar.display().to_string(),
                data_root: paths.data_root.display().to_string(),
                last,
            };
            let report = shutdown(state);
            let mut dto =
                StatusDto::refused(error, Some(paths), consent_granted(&paths.config_dir));
            append_kill_error(&mut dto, &report);
            dto
        }
    }
}

fn resolve_token() -> Result<String, ContractError> {
    if let Some(token) = token::resolve_platform(std::env::var("BRAIN_SHELL_TOKEN").ok()) {
        return Ok(token);
    }
    let token = generate_token()?;
    token::write_platform(&token).map_err(|_| ContractError::TokenUnavailable)?;
    Ok(token)
}

fn open_log(data_root: &Path) -> Result<File, ContractError> {
    let path = data_root.join(LOG_FILE);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| ContractError::Filesystem {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        ContractError::Filesystem {
            path: path.display().to_string(),
            reason: error.to_string(),
        }
    })?;
    Ok(file)
}

fn child_exit(state: &KernelState) -> Option<ExitStatus> {
    let mut guard = state
        .child
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .as_mut()
        .and_then(|child| child.try_wait().ok().flatten())
}

/// Append a kill failure to the refusal's diagnostic. The refusal's own
/// cause stays first; a failed-kill note is never substituted for it.
fn append_kill_error(dto: &mut StatusDto, report: &ShutdownReport) {
    let Some(kill) = &report.kill_error else {
        return;
    };
    dto.last_error = Some(match dto.last_error.take() {
        Some(last) => format!("{last}; the spawned sidecar could not be killed: {kill}"),
        None => format!("the spawned sidecar could not be killed: {kill}"),
    });
}
