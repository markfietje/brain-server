// The shell's Rust core: system integration ONLY — zero business logic
// (the kernel owns every rule; the webview talks the typed wire). Phase A
// carried the D7 token pass-through, which reads the OPERATOR'S OWN
// environment at runtime so the token never enters the JS bundle. Phase I
// adds `recall.rs`: the recall wire's BOUNDS and its reading of the
// kernel's answer. No fs, no shell, no http, no process plugins; NO unsafe.
//
// The core does NOT hold the wire. `capabilities/main.json` states the
// posture this file now matches: the webview reaches the kernel over the
// pinned loopback origin through the page's own fetch, and never through
// Tauri. What lives here is the part that must not live in a renderer or in
// Swift — the bounds, and the classification of what came back, including the
// refusal that must never render as "no results".
#![warn(clippy::all)]
#![deny(clippy::cargo)]
#![forbid(unsafe_code)]

pub mod forget;
pub mod recall;
pub mod verify;

mod token;

#[cfg(test)]
mod forget_tests;
#[cfg(test)]
mod recall_tests;
#[cfg(test)]
mod token_tests;
#[cfg(test)]
mod verify_tests;

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

/// III-2 — write the bearer to the login keychain. The value is trimmed and
/// bounded before any store is touched; the error is a stable code, never the
/// token. The store work runs on the blocking pool for the same reason the
/// read does.
#[tauri::command]
async fn store_kernel_token(token: String) -> Result<(), String> {
    let token = token::normalize(token).map_err(|e| e.code().to_string())?;
    tauri::async_runtime::spawn_blocking(move || token::write_platform(&token))
        .await
        .map_err(|_| "keychain_write_failed".to_string())?
        .map_err(|e| e.code().to_string())
}

/// III-2 — remove the stored bearer. Clearing an absent item is success.
#[tauri::command]
async fn clear_kernel_token() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(token::clear_platform)
        .await
        .map_err(|_| "keychain_clear_failed".to_string())?
        .map_err(|e| e.code().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            kernel_token,
            store_kernel_token,
            clear_kernel_token,
            recall_memory,
            shape_recall_command,
            verify_claim_command,
            shape_verify_command,
            forget_memory,
            shape_forget_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running the shell");
}
