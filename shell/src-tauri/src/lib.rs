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
pub mod verify;
// The direct channel's updater (III-4). EXCLUDED from the `mas` build: the
// store owns updates there (guideline 2.5.2), so the module, its pins, its
// commands and its plugin are all compiled out — an excision, not a runtime
// flag (PLAN_MAS_FORK §3).
#[cfg(not(feature = "mas"))]
pub mod updater;
// W-1 — the install core for the SERVICE posture. The same config boundary
// as the updater: the wizard's install surface is DIRECT-CHANNEL ONLY
// (2.4.5(ii)/(iii)), so the module, its pins and its commands are all
// compiled out of the `mas` build — an excision, not a runtime flag.
#[cfg(not(feature = "mas"))]
pub mod service;
// W-T1 — the detection core (D1–D5): the installer's REPORT of what the
// machine already has. Same config boundary: the wizard's proposal cards
// are DIRECT-CHANNEL ONLY, so the module, its pins and its command are
// compiled out of the `mas` build. The ONE production module whose source
// legally names the operator's label — reading the D1 plist to PROPOSE is
// the design; ZERO writes, pinned.
#[cfg(not(feature = "mas"))]
pub mod detect;

mod token;

#[cfg(all(test, not(feature = "mas")))]
mod detect_pins;
#[cfg(test)]
mod forget_pins;
#[cfg(test)]
mod recall_pins;
#[cfg(all(test, not(feature = "mas")))]
mod service_pins;
#[cfg(test)]
mod sidecar_pins;
#[cfg(test)]
mod token_pins;
#[cfg(all(test, not(feature = "mas")))]
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

/// Build the NATIVE macOS application menu (the operator's UX call, 2026-10-30:
/// "make the settings as native macOS as possible"). The HIG structure, in
/// order: the app submenu (About / Settings ⌘, / Services / Hide / Quit),
/// File (Close Window), Edit (the standard clipboard roles — WKWebView does
/// NOT wire copy/paste/select-all into the composer without them), View,
/// Window, Help. `Settings…` is a CUSTOM item (⌘, — the canonical macOS
/// preferences accelerator) because Tauri's predefined roles have no
/// settings entry; its selection arrives in the webview as `settings://open`
/// and the frame opens its preferences surface.
fn build_native_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::*;

    let settings = MenuItemBuilder::with_id("settings", "Settings…")
        .accelerator("CmdOrCtrl+,")
        .build(app)?;

    let app_submenu = SubmenuBuilder::new(app, "StewardMem")
        .about(None)
        .item(&settings)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let file_submenu = SubmenuBuilder::new(app, "File").close_window().build()?;

    let edit_submenu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_submenu = SubmenuBuilder::new(app, "View").fullscreen().build()?;

    let window_submenu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .separator()
        .close_window()
        .build()?;

    let help_submenu = SubmenuBuilder::new(app, "Help").build()?;

    MenuBuilder::new(app)
        .item(&app_submenu)
        .item(&file_submenu)
        .item(&edit_submenu)
        .item(&view_submenu)
        .item(&window_submenu)
        .item(&help_submenu)
        .build()
}

/// D7 / III-2 — the bearer token's runtime injection point. Read order: the
/// login keychain (service `com.stewardmem.shell/token`, account `default`), then
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

/// W-1 — the SERVICE posture's install (posture B). Consent, the port
/// decision, the generated LaunchAgent plist, the mode laws and the version
/// gate all live in `service.rs`; this edge only moves the work to the
/// blocking pool (launchctl and the fs are blocking work, not webview work).
#[cfg(not(feature = "mas"))]
#[tauri::command]
async fn service_install(app: tauri::AppHandle) -> Result<service::InstallDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<sidecar::KernelState>();
        service::service_install(&app, state.inner())
    })
    .await
    .map_err(|error| format!("service_install task failed: {error}"))
}

/// W-1 — the SERVICE posture's uninstall: bootout OUR label, remove OUR
/// plist, enumerate. The data root is OFFERED by the DTO, never deleted
/// here; clearing it is `service_remove_data`, its own named act.
#[cfg(not(feature = "mas"))]
#[tauri::command]
async fn service_uninstall(app: tauri::AppHandle) -> Result<service::UninstallDto, String> {
    tauri::async_runtime::spawn_blocking(move || service::service_uninstall(&app))
        .await
        .map_err(|error| format!("service_uninstall task failed: {error}"))
}

/// W-4 — the OFFERED service-data removal: deletes exactly the service root
/// the uninstall DTO named. A named act with a named path, never silent.
#[cfg(not(feature = "mas"))]
#[tauri::command]
async fn service_remove_data(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || service::service_remove_data(&app))
        .await
        .map_err(|error| format!("service_remove_data task failed: {error}"))?
        .map_err(|error| error.kind().to_string())
}

/// W-2 — the posture marker's reading: what the user CHOSE (the machine's
/// reality — launchd, the probe — is reported beside it by the caller).
#[cfg(not(feature = "mas"))]
#[tauri::command]
async fn service_posture(app: tauri::AppHandle) -> Result<service::PostureDto, String> {
    tauri::async_runtime::spawn_blocking(move || service::posture_load(&app))
        .await
        .map_err(|error| format!("service_posture task failed: {error}"))
}

/// W-2 — the posture choice, validated and saved. There is NO token
/// parameter: posture C never receives a credential through this app (the
/// operator pastes it into the remote kernel's own auth file).
#[cfg(not(feature = "mas"))]
#[tauri::command]
async fn store_service_posture(
    app: tauri::AppHandle,
    posture: String,
    url: Option<String>,
) -> Result<service::PostureDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        service::posture_store(&app, &posture, url.as_deref())
    })
    .await
    .map_err(|error| format!("store_service_posture task failed: {error}"))?
    .map_err(|error| error.kind().to_string())
}

/// W-T1 — the detection report the wizard's proposal cards consume. Every
/// card carries its evidence path; the offered sidecar for D5 is the app's
/// own sealed kernel (what posture B would serve). Read-only by law.
#[cfg(not(feature = "mas"))]
#[tauri::command]
async fn detect_cards(app: tauri::AppHandle) -> Result<detect::DetectDto, String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let home = app
            .path()
            .home_dir()
            .map_err(|error| format!("home_dir: {error}"))?;
        let offered = std::env::current_exe()
            .ok()
            .map(|exe| sidecar::resolve_sidecar_path(&exe));
        Ok(detect::detect_all(&home, offered.as_deref()))
    })
    .await
    .map_err(|error| format!("detect_cards task failed: {error}"))?
}

/// III-4 — the update CHECK. Read-only: it never installs and never writes.
/// The answer is a DTO, never a thrown error, and `unreachable` is its own
/// state — an endpoint that cannot be reached is never "you are up to date".
#[cfg(not(feature = "mas"))]
#[tauri::command]
async fn update_check(app: tauri::AppHandle) -> Result<updater::UpdateDto, String> {
    updater::check(&app).await
}

/// III-4 — the update INSTALL. The same gate, then the download, then the
/// install and the relaunch. A refusal never reaches the download.
#[cfg(not(feature = "mas"))]
#[tauri::command]
async fn update_install(app: tauri::AppHandle) -> Result<updater::UpdateDto, String> {
    updater::install(&app).await
}

/// The distribution channel this binary was built for (PLAN_MAS_FORK §3).
/// The renderer gates the direct channel's affordances on it — the store
/// edition renders no updater surface and never learns an update endpoint.
#[tauri::command]
fn build_channel() -> &'static str {
    if cfg!(feature = "mas") {
        "mas"
    } else {
        "direct"
    }
}

/// The optional Signal gateway module's local posture (PLAN_MAS_FORK §1):
/// DESCRIBE + DETECT, nothing more. The gateway is a SEPARATE, AGPL-licensed
/// process the user installs themselves from the website — this command
/// never installs, downloads, executes, or configures it (guideline
/// 2.4.5(iv)); it probes the gateway's documented loopback address with a
/// bounded TCP connect and reports what it found. A sandboxed app may
/// connect out to localhost (`network.client`), which is exactly this shape.
#[tauri::command]
fn signal_gateway_status() -> SignalGatewayStatus {
    // tools/signal-gateway/config.example.yaml: 127.0.0.1:8080, loopback-only
    // unless the operator deliberately widened it — the app probes the
    // documented default and nothing else.
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], 8080));
    let bound =
        std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(300))
            .is_ok();
    SignalGatewayStatus {
        present: bound,
        address: "127.0.0.1:8080".to_string(),
    }
}

/// The detect-only DTO: whether a gateway answered on the documented
/// loopback address. No version, no capability list, no content — the less
/// the surface says, the less there is to review (4.2.3(i): fully
/// functional without it).
#[derive(Clone, Debug, serde::Serialize)]
struct SignalGatewayStatus {
    present: bool,
    address: String,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .manage(sidecar::KernelState::default())
        .setup(|app| {
            // The native menu (HIG structure above) + the settings event:
            // the ⌘, selection crosses to the webview, where the frame owns
            // the preferences surface.
            let handle = app.handle().clone();
            let menu = build_native_menu(&handle)?;
            app.set_menu(menu)?;
            app.on_menu_event(move |_app, event| {
                if event.id().0 == "settings" {
                    use tauri::Emitter;
                    if let Err(error) = _app.emit("settings://open", ()) {
                        eprintln!("settings menu event could not reach the webview: {error}");
                    }
                }
            });
            Ok(())
        });
    // III-4 — the updater plugin, and NOTHING of it to the webview: the
    // capability grants no `updater:*` permission, so the renderer cannot
    // check or install anything. The Rust core owns both, and it reads the
    // pinned public key from the app's own configuration. ABSENT from the
    // `mas` build entirely (2.5.2: the store owns updates) — an excision,
    // not a runtime flag, which is why the registration lives on its own
    // statement and not in the chain above (a method chain cannot carry a
    // cfg gate).
    #[cfg(not(feature = "mas"))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    let app = builder
        .invoke_handler(tauri::generate_handler![
            kernel_token,
            store_kernel_token,
            clear_kernel_token,
            kernel_status,
            kernel_start,
            kernel_wire_status,
            build_channel,
            signal_gateway_status,
            #[cfg(not(feature = "mas"))]
            service_install,
            #[cfg(not(feature = "mas"))]
            service_uninstall,
            #[cfg(not(feature = "mas"))]
            service_remove_data,
            #[cfg(not(feature = "mas"))]
            service_posture,
            #[cfg(not(feature = "mas"))]
            store_service_posture,
            #[cfg(not(feature = "mas"))]
            detect_cards,
            #[cfg(not(feature = "mas"))]
            update_check,
            #[cfg(not(feature = "mas"))]
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
