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

/// The ONE environment variable this module reads: the endpoint override.
///
/// It moves WHERE the bytes come from. It cannot move WHO they are — the
/// public key is read from the app's own configuration, and there is no
/// environment variable, flag or command that changes it.
pub const ENDPOINT_ENV: &str = "BRAIN_SHELL_UPDATE_ENDPOINT";

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
pub fn decide(current_kernel: Option<&str>, manifest: &serde_json::Value) -> Decision {
    // NOT IMPLEMENTED YET — the pins in `updater_tests.rs` are RED against
    // this, and the gate lands in the next commit.
    let _ = (current_kernel, manifest);
    Decision::Allowed {
        offered_kernel: String::new(),
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

/// What the webview is told about the update.
///
/// A DTO, never a thrown error: the same shape as III-3's `StatusDto`. The
/// states are `current` (nothing newer), `available` (an update the gate
/// allowed), `refused` (the gate refused it) and `unreachable` (the endpoint
/// could not be reached or could not be parsed) — and `unreachable` is its
/// OWN state, because an endpoint that cannot be reached is never "you are up
/// to date".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

/// Verify an update artifact's bytes against the key the app ships.
///
/// The plugin verifies on download and again on install; this is the third
/// check, and it is the one that NAMES the refusal instead of reporting a
/// bare error string.
pub fn verify_artifact(update: &tauri_plugin_updater::Update, bytes: &[u8]) -> Result<(), String> {
    // NOT IMPLEMENTED YET — the pin in `updater_tests.rs` is RED against
    // this, and the verification lands in the next commit.
    let _ = (update, bytes);
    Ok(())
}
