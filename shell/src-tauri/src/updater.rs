//! III-4 — the updater's gate: **where the bytes come from, and whose they are**.
//!
//! Two facts, kept apart, and the whole round is the distance between them:
//!
//! * **WHERE** — the manifest, the URL and the announced version are UNTRUSTED
//!   input. They steer where the bytes come from and nothing else. The
//!   endpoint may be overridden at runtime (`BRAIN_SHELL_UPDATE_ENDPOINT`),
//!   and that override moves WHERE, never WHO.
//! * **WHO** — the minisign signature against the ONE public key pinned in
//!   the app's own configuration is the only thing that decides whose bytes
//!   they are. There is **no path, anywhere**, to change that key at runtime:
//!   it is read from the app's config and from nowhere else.
//!
//! The update replaces the whole `.app` — **including the bundled kernel
//! sidecar** — so an update changes the kernel too. The gate therefore
//! refuses a release whose manifest does not vouch for BOTH the kernel this
//! app carries and the kernel the release offers. A manifest missing either
//! field is refused (fail closed), never "assume current". This composes
//! with III-3: the shell refuses a kernel it cannot vouch for, and now also
//! refuses to *acquire* one.
//!
//! The webview is granted **no** updater permission
//! (`capabilities/main.json`): the Rust core owns the check and the install,
//! and the webview receives a DTO. There is no second transport (the plugin
//! owns the HTTP), no npm package, and no `unsafe`.
//!
//! Two version axes, kept apart: the shell's own semver (`0.1.0` → `0.1.1`)
//! and the kernel's version, which travels in the manifest and never in the
//! app version. A kernel bump does not force a shell release, and a shell
//! release does not pretend to be a kernel release.

use serde::Serialize;
use tauri::Manager;
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};

use crate::sidecar;

/// The ONE environment variable this module reads: the endpoint override.
///
/// It moves WHERE the bytes come from. It cannot move WHO they are — the
/// public key is read from the app's own configuration, and there is no
/// environment variable, flag or command that changes it.
pub const ENDPOINT_ENV: &str = "BRAIN_SHELL_UPDATE_ENDPOINT";

/// The ceiling on one check.
///
/// The plugin's builder has no timeout by default, so an endpoint that
/// accepts the connection and then says nothing would leave the panel
/// spinning forever. A check that cannot finish must END, and end as
/// `unreachable` — which is its own state, never "you are up to date".
pub const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// What the gate decided about an offered release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The release may be installed.
    Allowed { offered_kernel: String },
    /// The release may not. `kind` names WHICH law refused it; `expected`
    /// and `observed` name the versions involved whenever a version is
    /// involved at all (III-3's law, unchanged).
    Refused {
        kind: &'static str,
        expected: Option<String>,
        observed: Option<String>,
    },
}

/// The gate's decision, given the kernel version THIS app carries and the
/// manifest the endpoint answered with.
///
/// The arms, in the order they are decided:
///
/// 1. no `kernel_version` → refused (`no_kernel_version`);
/// 2. no `kernel_versions`, or an empty list → refused
///    (`no_known_kernel_versions`);
/// 3. this app's own kernel version is unknown → refused
///    (`unknown_local_kernel`);
/// 4. this app's kernel version ∉ `kernel_versions` → refused
///    (`unknown_local_kernel`), naming both;
/// 5. the offered `kernel_version` ∉ `kernel_versions` → refused
///    (`unknown_offered_kernel`), naming both;
/// 6. otherwise → allowed, carrying the offered kernel version.
///
/// A refusal names BOTH versions whenever it names one: `expected` is what
/// the release vouches for, `observed` is what was found.
pub fn decide(current_kernel: Option<&str>, manifest: &serde_json::Value) -> Decision {
    let offered = manifest
        .get("kernel_version")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let Some(offered) = offered else {
        return Decision::Refused {
            kind: "no_kernel_version",
            expected: None,
            observed: None,
        };
    };

    let known: Vec<String> = manifest
        .get("kernel_versions")
        .and_then(serde_json::Value::as_array)
        .map(|versions| {
            versions
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    if known.is_empty() {
        return Decision::Refused {
            kind: "no_known_kernel_versions",
            expected: None,
            observed: None,
        };
    }
    let expected = Some(known.join(", "));

    let Some(local) = current_kernel
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Decision::Refused {
            kind: "unknown_local_kernel",
            expected: None,
            observed: None,
        };
    };

    if !known.iter().any(|version| version == local) {
        return Decision::Refused {
            kind: "unknown_local_kernel",
            expected,
            observed: Some(local.to_string()),
        };
    }

    if !known.iter().any(|version| version == offered) {
        return Decision::Refused {
            kind: "unknown_offered_kernel",
            expected,
            observed: Some(offered.to_string()),
        };
    }

    Decision::Allowed {
        offered_kernel: offered.to_string(),
    }
}

/// The ONE public key this app verifies an update with.
///
/// It is read from the app's own configuration and from nowhere else. The
/// endpoint may be overridden at runtime; the key may not — that is the
/// difference between WHERE and WHO.
pub fn pinned_pubkey(plugin_config: &serde_json::Value) -> Option<String> {
    plugin_config
        .get("pubkey")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

/// The app's own updater configuration, as the compiled app carries it.
fn plugin_config(app: &tauri::AppHandle) -> Option<&serde_json::Value> {
    app.config().plugins.0.get("updater")
}

/// The kernel version THIS app carries, read from the sealed provenance
/// III-3's refuse law already uses — never from `package.json`, never from
/// the app version, and never from the environment.
///
/// `None` means the record is missing or unreadable, and the gate then
/// refuses: an app that cannot say which kernel it carries must not accept
/// a release that replaces it.
pub fn embedded_kernel_version(app: &tauri::AppHandle) -> Option<String> {
    let resource_dir = app.path().resource_dir().ok()?;
    let provenance = sidecar::load_provenance(&resource_dir.join(sidecar::PROVENANCE_REL)).ok()?;
    Some(provenance.kernel_version)
}

/// The endpoint override, if one is set. This is the module's ONLY read of
/// the environment.
fn endpoint_override() -> Option<String> {
    std::env::var(ENDPOINT_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// What the webview is told about the update.
///
/// A DTO, never a thrown error: the same shape as III-3's `StatusDto`. The
/// states are `current` (nothing newer), `available` (an update the gate
/// allowed), `refused` (the gate refused it) and `unreachable` (the endpoint
/// could not be reached or could not be parsed) — and `unreachable` is its
/// OWN state, because an endpoint that cannot be reached is never "you are up
/// to date".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDto {
    pub state: String,
    pub kind: Option<String>,
    pub current_version: String,
    pub current_kernel: Option<String>,
    pub offered_version: Option<String>,
    pub offered_kernel: Option<String>,
    pub expected: Option<String>,
    pub observed: Option<String>,
    pub last_error: Option<String>,
}

impl UpdateDto {
    fn new(current_version: &str, current_kernel: Option<&str>) -> Self {
        Self {
            state: "current".to_string(),
            kind: None,
            current_version: current_version.to_string(),
            current_kernel: current_kernel.map(str::to_string),
            offered_version: None,
            offered_kernel: None,
            expected: None,
            observed: None,
            last_error: None,
        }
    }

    fn unreachable(mut self, error: &str) -> Self {
        self.state = "unreachable".to_string();
        self.last_error = Some(error.to_string());
        self
    }

    fn refused(
        mut self,
        kind: &str,
        expected: Option<String>,
        observed: Option<String>,
        detail: Option<String>,
    ) -> Self {
        self.state = "refused".to_string();
        self.kind = Some(kind.to_string());
        self.expected = expected.clone();
        self.observed = observed.clone();
        // The refusal says what it found and what it expected, in one
        // sentence — the same shape as III-3's "Expected version / Reported
        // version". A refusal that names ONE version names the other: the
        // comparison is the whole of the reason.
        self.last_error = Some(match (expected, observed) {
            (Some(expected), Some(observed)) => {
                format!("{kind}: expected {expected}, found {observed}")
            }
            _ => detail.unwrap_or_else(|| kind.to_string()),
        });
        self
    }

    fn available(mut self, offered_version: &str, offered_kernel: &str) -> Self {
        self.state = "available".to_string();
        self.offered_version = Some(offered_version.to_string());
        self.offered_kernel = Some(offered_kernel.to_string());
        self
    }
}

/// The updater, with the endpoint override applied when there is one.
///
/// The override moves WHERE the manifest is fetched from. It never touches
/// the public key: the key comes from the app's configuration, and the
/// builder inherits it.
fn updater_for(app: &tauri::AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let builder = app.updater_builder();
    // The trust anchor is named in ONE place: the app's own configuration.
    // It is read here and passed to the builder explicitly, so the chain
    // "config -> this module -> the plugin's verifier" has no other link in
    // it — no environment variable, no flag, no second source.
    let pubkey = plugin_config(app)
        .and_then(pinned_pubkey)
        .ok_or_else(|| "the app's configuration carries no updater public key".to_string())?;
    let builder = builder.pubkey(pubkey);
    let builder = builder.timeout(CHECK_TIMEOUT);
    let builder = match endpoint_override() {
        Some(endpoint) => {
            let url = tauri::Url::parse(&endpoint)
                .map_err(|_| "the endpoint override is not a URL".to_string())?;
            builder
                .endpoints(vec![url])
                .map_err(|error| format!("the endpoint override was refused: {error}"))?
        }
        None => builder,
    };
    builder.build().map_err(|error| error.to_string())
}

/// `update_check` — ask the endpoint what there is, and apply the gate.
///
/// It never installs anything and never writes anything.
pub async fn check(app: &tauri::AppHandle) -> Result<UpdateDto, String> {
    let current_version = app.package_info().version.to_string();
    let current_kernel = embedded_kernel_version(app);
    let dto = UpdateDto::new(&current_version, current_kernel.as_deref());

    let updater = match updater_for(app) {
        Ok(updater) => updater,
        Err(error) => return Ok(dto.unreachable(&error)),
    };

    let update = match updater.check().await {
        Ok(Some(update)) => update,
        // Nothing newer: the endpoint answered, and the answer is "no".
        Ok(None) => return Ok(dto),
        Err(error) => return Ok(dto.unreachable(&error.to_string())),
    };

    let offered_version = update.version.clone();
    match decide(current_kernel.as_deref(), &update.raw_json) {
        Decision::Refused {
            kind,
            expected,
            observed,
        } => Ok(dto.refused(kind, expected, observed, None)),
        Decision::Allowed { offered_kernel } => {
            Ok(dto.available(&offered_version, &offered_kernel))
        }
    }
}

/// `update_install` — the same gate, then the download, then the install and
/// the relaunch.
///
/// A refusal NEVER reaches the download: the gate runs first, and a release
/// the shell cannot vouch for is not fetched at all.
pub async fn install(app: &tauri::AppHandle) -> Result<UpdateDto, String> {
    let current_version = app.package_info().version.to_string();
    let current_kernel = embedded_kernel_version(app);
    let dto = UpdateDto::new(&current_version, current_kernel.as_deref());

    let updater = match updater_for(app) {
        Ok(updater) => updater,
        Err(error) => return Ok(dto.unreachable(&error)),
    };

    let update = match updater.check().await {
        Ok(Some(update)) => update,
        Ok(None) => return Ok(dto),
        Err(error) => return Ok(dto.unreachable(&error.to_string())),
    };

    // The gate runs FIRST: a release the shell cannot vouch for is never
    // fetched at all.
    match decide(current_kernel.as_deref(), &update.raw_json) {
        Decision::Refused {
            kind,
            expected,
            observed,
        } => return Ok(dto.refused(kind, expected, observed, None)),
        Decision::Allowed { .. } => {}
    }

    // The plugin verifies the bytes on download and again on install; this
    // is the third check, and it is the one that NAMES the refusal instead
    // of reporting a bare error string.
    let bytes = match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let message = download_error(&error);
            // BOTH arms move `dto` — a refusal and an unreachable are
            // different renderings of the same DTO — so this is an `if let`,
            // not a `map_or_else` (whose two closures would both move it).
            let dto = if let Some(kind) = refusal_kind(&error) {
                dto.refused(kind, None, None, Some(message))
            } else {
                dto.unreachable(&message)
            };
            return Ok(dto);
        }
    };
    if let Err(error) = verify_artifact(&update, &bytes) {
        return Ok(dto.refused("signature_rejected", None, None, Some(error)));
    }
    if let Err(error) = update.install(bytes) {
        return Ok(dto.unreachable(&install_error(&error)));
    }

    // The new bundle is in place; the app replaces itself with it.
    app.restart();
}

/// Verify an update artifact's bytes against the key the app ships.
///
/// The plugin verifies on download and again on install; this is the third
/// check, and it is the one that NAMES the refusal instead of reporting a
/// bare error string.
pub fn verify_artifact(update: &Update, bytes: &[u8]) -> Result<(), String> {
    update.verify(bytes).map_err(|error| error.to_string())
}

/// The refusal a plugin error IS, when it is one.
///
/// A signature that does not verify is a REFUSAL — the artifact is not the
/// pinned key's — and not an unreachable endpoint. Rendering it as
/// "unreachable" would send the operator looking at the network instead of
/// at the thing that is actually wrong.
fn refusal_kind(error: &UpdaterError) -> Option<&'static str> {
    match error {
        UpdaterError::Minisign(_) => Some("signature_rejected"),
        UpdaterError::SignedVersionMismatch { .. } => Some("signed_version_mismatch"),
        _ => None,
    }
}

/// The plugin's download error, said in the app's own words.
///
/// A minisign failure is the signature: the artifact was not signed by the
/// key this app pins. It is named as such, because "minisign error" is not
/// something an operator can act on.
fn download_error(error: &UpdaterError) -> String {
    match error {
        UpdaterError::Minisign(_) => "the artifact's signature is not the pinned key's".to_string(),
        UpdaterError::SignedVersionMismatch { signed, announced } => format!(
            "the artifact was signed for version {signed}, but the endpoint announced {announced}"
        ),
        other => other.to_string(),
    }
}

/// The plugin's install error, said in the app's own words.
fn install_error(error: &UpdaterError) -> String {
    match error {
        UpdaterError::Minisign(_) => "the artifact's signature is not the pinned key's".to_string(),
        UpdaterError::SignedVersionMismatch { signed, announced } => format!(
            "the artifact was signed for version {signed}, but the endpoint announced {announced}"
        ),
        other => other.to_string(),
    }
}
