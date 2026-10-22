// The shell's Rust core: system integration ONLY — zero business logic
// (the kernel owns every rule; the webview talks the typed wire). Phase A
// carried the D7 token pass-through, which reads the OPERATOR'S OWN
// environment at runtime so the token never enters the JS bundle. Phase I
// adds `recall.rs`: the recall wire's BOUNDS and its reading of the
// kernel's answer. III-3 adds `sidecar.rs`, the ONE module that reverses the
// fs/process half of the posture: it owns the data root, the consent marker,
// the auth file, the log, and the bundled kernel's lifecycle. It adds no
// HTTP client (the health probe is a raw loopback `TcpStream`) and no Tauri
// plugin; there is NO unsafe.
//
// The core does NOT hold the wire. `capabilities/main.json` states the
// posture this file now matches: the webview reaches the kernel over the
// pinned loopback origin through the page's own fetch, and never through
// Tauri. What lives here is the part that must not live in a renderer or in
// Swift — the bounds, the classification of what came back (including the
// refusal that must never render as "no results"), and the kernel contract's
// detect/start/refuse edges.
#![warn(clippy::all)]
#![deny(clippy::cargo)]
#![forbid(unsafe_code)]

pub mod forget;
pub mod recall;
pub mod sidecar;
pub mod updater;
pub mod verify;

mod token;

#[cfg(test)]
mod forget_pins;
#[cfg(test)]
mod recall_pins;
#[cfg(test)]
mod sidecar_pins;
#[cfg(test)]
mod token_pins;
#[cfg(test)]
mod updater_pins;
#[cfg(test)]
mod verify_pins;

pub use forget::{
    AuthorizedForget, ForgetDoc, ForgetOutcome, ForgetRefusal, ForgetRequest, ForgetResult,
    NoSuchRow, Tombstoned, WireForget, authorize as authorize_forget, forget_memory, shape_forget,
    shape_forget_command,
};

pub use recall::{
    AuthorizedRecall, DEFAULT_LIMIT, Decision, MAX_DOMAIN_BYTES, MAX_LIMIT, MAX_QUERY_BYTES,
    MIN_LIMIT, Passage, RecallAnswer, RecallDoc, RecallOutcome, RecallQuery, RecallRefusal,
    Refused, Searched, Unreachable, WireExchange, authorize, recall_memory, shape_recall,
    shape_recall_command,
};
pub use verify::{
    AuthorizedVerify, MAX_CLAIM_CHARS, MAX_MATCH_RANGES, MIN_CHUNK_ID, NoSuchChunk, Verdict,
    VerifyDecision, VerifyDoc, VerifyOutcome, VerifyRefusal, VerifyRequest, WireVerify,
    authorize as authorize_verify, shape_verify, shape_verify_command, verify_claim_command,
};

use tauri::Manager;

/// D7 / III-2 — the bearer token's runtime injection point. Read order: the
/// login keychain (service `app.brain.shell/token`, account `default`), then
/// the shell's own `BRAIN_SHELL_TOKEN`, absent last. Once handed over it is
/// held in JS memory only (see src/lib/api/token.ts); the webview never logs,
/// renders or persists it. The kernel's operator credentials (`AUTH_TOKEN`,
/// `BRAIN_TOKEN`, and their files) are NOT read here: that is the privilege
/// boundary this shell keeps.
///
/// The keychain read runs on the blocking pool — a login-keychain access can
/// wait on a system prompt, and a sync command would freeze the UI.
#[tauri::command]
async fn kernel_token() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        token::resolve_platform(std::env::var("BRAIN_SHELL_TOKEN").ok())
    })
    .await
    .ok()
    .flatten()
}

/// III-2/III-3 — write the bearer to the login keychain AND rotate the
/// running kernel's token file. The keychain is the SHELL's bearer; a
/// running kernel reads ITS bearer from `<data_root>/auth-token`, and the
/// kernel's `TokenStore` reloads that file when the path's mtime advances.
/// Writing only the keychain left the running kernel on the OLD token until
/// the app restarted — the first pass's OWED. The rotation's outcome comes
/// back to the caller so the Settings panel can say what actually happened;
/// a failed rotation is never reported as a saved-and-live token.
#[tauri::command]
async fn store_kernel_token(
    app: tauri::AppHandle,
    token: String,
) -> Result<sidecar::RotationDto, String> {
    let token = token::normalize(token).map_err(|e| e.code().to_string())?;
    let for_keychain = token.clone();
    tauri::async_runtime::spawn_blocking(move || token::write_platform(&for_keychain))
        .await
        .map_err(|_| "keychain_write_failed".to_string())?
        .map_err(|e| e.code().to_string())?;

    // The rotation is fs work, so it runs on the blocking pool too.
    let rotated =
        tauri::async_runtime::spawn_blocking(move || sidecar::rotate_running_auth(&app, &token))
            .await
            .map_err(|_| "rotation_task_failed".to_string())?;

    // A failed rotation is REPORTED, not propagated: the DTO carries the
    // refusal, so the caller renders it rather than losing the state.
    Ok(rotated.unwrap_or_else(|error| sidecar::RotationDto {
        kernel_synced: false,
        data_root_missing: false,
        error: Some(error.kind().to_string()),
    }))
}

/// III-2 — remove the stored bearer. Clearing an absent item is success.
#[tauri::command]
async fn clear_kernel_token() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(token::clear_platform)
        .await
        .map_err(|_| "keychain_clear_failed".to_string())?
        .map_err(|e| e.code().to_string())
}

/// III-3 — detect. Reads the sealed provenance and probes `/health`; it never
/// spawns, never creates a directory, and never kills a listener it did not
/// start. A refusal is a DTO the gate renders, not a thrown error.
#[tauri::command]
async fn kernel_status(app: tauri::AppHandle) -> Result<sidecar::StatusDto, String> {
    tauri::async_runtime::spawn_blocking(move || sidecar::status(&app))
        .await
        .map_err(|error| format!("kernel_status task failed: {error}"))
}

/// III-3 — start (consent → token → auth file → spawn → bounded wait). The
/// wait runs on the blocking pool, NOT in the webview: the 10 s bound and the
/// timeout kill are only enforceable on this side.
#[tauri::command]
async fn kernel_start(app: tauri::AppHandle, consent: bool) -> Result<sidecar::StatusDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<sidecar::KernelState>();
        sidecar::start(&app, state.inner(), consent, sidecar::WAIT_MS)
    })
    .await
    .map_err(|error| format!("kernel_start task failed: {error}"))
}

/// III-4 — the WIRE diagnosis: what the webview would experience with the
/// running kernel, asked of the kernel itself over the raw loopback stream.
///
/// The renderer cannot tell a blocked response from a rejected bearer from a
/// dead port — the browser engine reports all three as one network failure —
/// so the shell asks, and NAMES the fault: the origin is not allowed, the
/// credential is rejected, no credential is configured, or nothing is
/// listening. Read-only: it never resolves, generates or stores a credential.
#[tauri::command]
async fn kernel_wire_status() -> Result<sidecar::WireDto, String> {
    tauri::async_runtime::spawn_blocking(|| sidecar::wire_status(sidecar::PORT))
        .await
        .map_err(|_| "wire_status task failed".to_string())
}

/// III-4 — the update CHECK. Read-only: it never installs and never writes.
/// The answer is a DTO, never a thrown error, and `unreachable` is its own
/// state — an endpoint that cannot be reached is never "you are up to date".
#[tauri::command]
async fn update_check(app: tauri::AppHandle) -> Result<updater::UpdateDto, String> {
    updater::check(&app).await
}

/// III-4 — the update INSTALL. The same gate, then the download, then the
/// install and the relaunch. A refusal never reaches the download.
#[tauri::command]
async fn update_install(app: tauri::AppHandle) -> Result<updater::UpdateDto, String> {
    updater::install(&app).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .manage(sidecar::KernelState::default())
        // III-4 — the updater plugin, and NOTHING of it to the webview: the
        // capability grants no `updater:*` permission, so the renderer cannot
        // check or install anything. The Rust core owns both, and it reads
        // the pinned public key from the app's own configuration.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            kernel_token,
            store_kernel_token,
            clear_kernel_token,
            kernel_status,
            kernel_start,
            kernel_wire_status,
            update_check,
            update_install,
            recall_memory,
            shape_recall_command,
            verify_claim_command,
            shape_verify_command,
            forget_memory,
            shape_forget_command
        ])
        .build(tauri::generate_context!())
        .expect("error while building the shell");
    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            // Only the child this process spawned is held; an attached kernel
            // is not ours to stop. A kill failure is the one thing the exit
            // path can still say out loud.
            if let Some(error) = sidecar::shutdown_app(handle) {
                eprintln!("brain-shell: the spawned kernel could not be killed: {error}");
            }
        }
    });
}
