//! W-T1 — the detection core (D1–D5): the installer's first screen is not a
//! form, it is a REPORT of what the machine already has. Every detection is
//! a documented-path read; nothing is guessed, nothing scans the disk, and
//! there is **ZERO network, ZERO privilege, ZERO writes** — the last one
//! pinned by a source-scan that refuses the module to so much as name a
//! write API. The Tools doc's own design makes this the ONE module whose
//! source legally names the operator's `com.brain.server`: reading the D1
//! plist to PROPOSE is the design; the collision law forbids WRITES, not
//! reads.
//!
//! The evidence law: every card carries the path(s) it actually read,
//! because a proposal the user cannot verify is a guess wearing a badge. A
//! card with no evidence found renders "not found — nothing touched", never
//! a silent absence.
//!
//! D3 enumerates names/sizes/mtimes and NEVER opens the databases — the
//! contents are the user's; the type shape carries no content field at all.
//! D4 reuses the sidecar's own probe (one probe implementation, every
//! consumer). D5 compares versions so the card can name BOTH.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;

use crate::sidecar::{self, ProbeOutcome};

/// The operator's service label — the ONE production module allowed to name
/// it, and only to READ its plist. The W-1 pin refuses this string anywhere
/// in `service.rs`'s write paths; here it is the documented D1 location the
/// Tools doc measured (`~/Library/LaunchAgents/com.brain.server.plist` →
/// `ProgramArguments[0]`).
const OPERATOR_LABEL: &str = "com.brain.server";

/// The D1 plist, by its documented path and nothing else. The pin holds this
/// to be the module's ONLY LaunchAgents construct.
fn d1_plist_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{OPERATOR_LABEL}.plist"))
}

/// The OpenClaw config the D2 card reads (the measured path).
fn d2_config_path(home: &Path) -> PathBuf {
    home.join(".openclaw/openclaw.json")
}

/// The D3 workspace directory (the measured path; the enumeration never
/// opens what it finds).
fn d3_workspace_dir(home: &Path) -> PathBuf {
    home.join(".openclaw/workspace")
}

// ── the cards ───────────────────────────────────────────────────────────────

/// The evidence every card carries: the path(s) the detection actually read.
/// A claim without a path is a lie the pin refuses.
pub trait Evidence {
    fn evidence(&self) -> String;
}

/// D1 — an existing brain-server installation: which binary the operator's
/// service manages, its version, running-or-not.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingInstallDto {
    pub binary_path: String,
    pub version: Option<String>,
    pub running: bool,
    pub evidence: String,
}

/// D2 — the OpenClaw integration: which config carries it, enabled or not.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawDto {
    pub plugin_enabled: bool,
    pub evidence: String,
}

/// D3 — ONE workspace database's listing: name, size, mtime. There is NO
/// content field — the installer never opens the user's databases, and the
/// type cannot carry what it must not read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDbDto {
    pub name: String,
    pub size_bytes: u64,
    pub modified_epoch: u64,
}

/// D3 — the workspace memory card: N databases, total size, last-touched.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDto {
    pub databases: Vec<WorkspaceDbDto>,
    pub total_bytes: u64,
    pub evidence: String,
}

/// D4 — a kernel already answering on 8765: the probe's verdict, the
/// version, and the holder sentence — the same words the install edge
/// refuses with.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KernelOnPortDto {
    pub running: bool,
    pub version: Option<String>,
    pub holder: Option<String>,
    pub evidence: String,
}

/// D5 — one binary's update fact: installed X → offered Y, both named.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDto {
    pub binary: String,
    pub installed_version: Option<String>,
    pub offered_version: Option<String>,
    pub evidence: String,
}

/// The full report: what the machine already has. Every absent card is
/// `None` — the renderer shows "not found — nothing touched", never a
/// silent absence.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DetectDto {
    pub existing: Option<ExistingInstallDto>,
    pub openclaw: Option<OpenClawDto>,
    pub workspace: Option<WorkspaceDto>,
    pub kernel_on_port: KernelOnPortDto,
    pub updates: Vec<UpdateDto>,
}

// ── D1 — the existing installation ─────────────────────────────────────────

/// The D1 read: the documented plist path, `ProgramArguments[0]`, the
/// binary's `--version`. The ONLY read of the operator's service this
/// module performs. The running fact is the CALLER's probe verdict (D4's,
/// so one probe serves both cards — a detector that re-probes per card is
/// a second transport wearing the first one's name).
pub fn detect_existing_install(home: &Path, kernel_running: bool) -> Option<ExistingInstallDto> {
    let plist_path = d1_plist_path(home);
    let value = plist::Value::from_file(&plist_path).ok()?;
    let binary = value
        .as_dictionary()?
        .get("ProgramArguments")?
        .as_array()?
        .first()?
        .as_string()?
        .to_string();
    let evidence = format!("{} → ProgramArguments[0] = {binary}", plist_path.display());
    let version = bounded_version(&binary);
    Some(ExistingInstallDto {
        binary_path: binary,
        version,
        running: kernel_running,
        evidence,
    })
}

/// `--version` exec, bounded: a hung program is an UNKNOWN version, never a
/// hung detection. Read-only; the child's stdout is the only thing read.
fn bounded_version(program: &str) -> Option<String> {
    let mut child = Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut out = String::new();
                use std::io::Read;
                if child
                    .stdout
                    .take()
                    .is_some_and(|mut s| s.read_to_string(&mut out).is_ok())
                {
                    let version = out.split_whitespace().last().unwrap_or_default();
                    if version.starts_with(|c: char| c.is_ascii_digit()) {
                        return Some(version.to_string());
                    }
                }
                return None;
            }
            Ok(Some(_)) => return None,
            Ok(None) if std::time::Instant::now() >= deadline => {
                child.kill().ok();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
}

// ── D2 — the OpenClaw integration ───────────────────────────────────────────

/// The D2 read: the config parses, the brain-server plugin block carries its
/// enabled flag. The plugin's env-file reference stays in the config file —
/// this module reads the config, never the env it points at.
pub fn detect_openclaw(home: &Path) -> Option<OpenClawDto> {
    let config_path = d2_config_path(home);
    let text = std::fs::read_to_string(&config_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let enabled = value
        .get("plugins")
        .and_then(|plugins| plugins.get("entries"))
        .and_then(|entries| entries.get(OPERATOR_LABEL))
        .and_then(|plugin| plugin.get("enabled"))
        .and_then(|flag| flag.as_bool())
        .unwrap_or(false);
    Some(OpenClawDto {
        plugin_enabled: enabled,
        evidence: format!(
            "{} → plugins.entries.{OPERATOR_LABEL}",
            config_path.display()
        ),
    })
}

// ── D3 — the workspace memory ───────────────────────────────────────────────

/// The D3 enumeration: names, sizes, mtimes — metadata only. The databases
/// are NEVER opened: no `fs::read`, no contents field, nothing that could
/// carry the user's memory out of the listing.
pub fn detect_workspace(home: &Path) -> Option<WorkspaceDto> {
    let dir = d3_workspace_dir(home);
    let mut databases = Vec::new();
    for entry in std::fs::read_dir(&dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy().to_string();
        let is_db = name.ends_with(".db") || name.contains(".sqlite");
        if !is_db || !path.is_file() {
            continue;
        }
        let metadata = entry.metadata().ok()?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        databases.push(WorkspaceDbDto {
            name,
            size_bytes: metadata.len(),
            modified_epoch: modified,
        });
    }
    if databases.is_empty() {
        return None;
    }
    databases.sort_by_key(|db| std::cmp::Reverse(db.modified_epoch));
    let total = databases.iter().map(|db| db.size_bytes).sum();
    Some(WorkspaceDto {
        databases,
        total_bytes: total,
        evidence: format!(
            "{} — names, sizes, mtimes; the contents are never opened",
            dir.display()
        ),
    })
}

// ── D4 — a kernel on 8765 ───────────────────────────────────────────────────

/// The D4 probe: the SAME probe the app and the install edge use —
/// `sidecar::probe`, never a second transport. The holder sentence is the
/// refusal's own words, so the card and the refusal tell one truth.
pub fn detect_kernel_on_port() -> KernelOnPortDto {
    let evidence = format!(
        "GET /health on 127.0.0.1:{} — the app's own probe",
        sidecar::PORT
    );
    match sidecar::probe(
        sidecar::PORT,
        Duration::from_millis(sidecar::PROBE_TIMEOUT_MS),
    ) {
        ProbeOutcome::Probe(probe) => KernelOnPortDto {
            running: true,
            version: Some(probe.api_version),
            holder: Some(
                "a kernel that answers as a Brain kernel, not started by this app".to_string(),
            ),
            evidence,
        },
        ProbeOutcome::Inconsistent { header, body } => KernelOnPortDto {
            running: true,
            version: Some(header),
            holder: Some(format!(
                "a Brain kernel whose versions disagree ({body}), not started by this app"
            )),
            evidence,
        },
        ProbeOutcome::Foreign => KernelOnPortDto {
            running: true,
            version: None,
            holder: Some("a listener that did not answer as a Brain kernel".to_string()),
            evidence,
        },
        ProbeOutcome::Unanswered => KernelOnPortDto {
            running: true,
            version: None,
            holder: Some("a listener that answered nothing".to_string()),
            evidence,
        },
        ProbeOutcome::NotRunning => KernelOnPortDto {
            running: false,
            version: None,
            holder: None,
            evidence,
        },
    }
}

// ── D5 — the version compare ────────────────────────────────────────────────

/// D5: the offered binary against the installed one, BOTH versions named.
/// The offered path is what the DMG carries; the installed path is D1's
/// `ProgramArguments[0]`.
pub fn detect_update(offered: &Path, installed: &str) -> Option<UpdateDto> {
    let offered_version = bounded_version(&offered.display().to_string())?;
    let installed_version = bounded_version(installed);
    Some(UpdateDto {
        binary: offered.file_name().map_or_else(
            || offered.display().to_string(),
            |name| name.to_string_lossy().to_string(),
        ),
        installed_version,
        offered_version: Some(offered_version),
        evidence: format!(
            "{installed} (--version) vs {} (--version)",
            offered.display()
        ),
    })
}

/// Semver ordering for the update card: numeric components, left to right,
/// missing components are zero; when the cores are equal a pre-release
/// loses to its release — the same convention the kernel's own version
/// gate holds.
pub fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    fn components(version: &str) -> Vec<u64> {
        let core = version.split(['-', '+']).next().unwrap_or(version);
        core.split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    }
    fn is_prerelease(version: &str) -> bool {
        version.split_once('-').is_some()
    }
    let (left, right) = (components(a), components(b));
    let width = left.len().max(right.len());
    for index in 0..width {
        let l = left.get(index).copied().unwrap_or(0);
        let r = right.get(index).copied().unwrap_or(0);
        match l.cmp(&r) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    match (is_prerelease(a), is_prerelease(b)) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    }
}

// ── the report ──────────────────────────────────────────────────────────────

/// The whole report, one call. `offered_sidecar` is the kernel binary the
/// app (or the DMG) carries — the D5 compare's offered side; `None` skips
/// D5 (no offered binary, no update card).
pub fn detect_all(home: &Path, offered_sidecar: Option<&Path>) -> DetectDto {
    let kernel_on_port = detect_kernel_on_port();
    let existing = detect_existing_install(home, kernel_on_port.running);
    let openclaw = detect_openclaw(home);
    let workspace = detect_workspace(home);
    let updates = match (&existing, offered_sidecar) {
        (Some(installed), Some(offered)) => detect_update(offered, &installed.binary_path)
            .filter(|update| {
                // An update is proposed only when BOTH versions are known
                // and differ — an unknown installed version is not a
                // proposal, it is a guess.
                update.installed_version.is_some()
                    && update.installed_version != update.offered_version
            })
            .into_iter()
            .collect(),
        _ => Vec::new(),
    };
    DetectDto {
        existing,
        openclaw,
        workspace,
        kernel_on_port,
        updates,
    }
}

/// The card sentence for a NOT-found detection — the no-silent-absence law:
/// the renderer shows this, never an empty hole.
pub const NOT_FOUND_SENTENCE: &str = "not found — nothing touched";
