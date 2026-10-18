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

#[cfg(test)]
mod forget_tests;
#[cfg(test)]
mod recall_tests;
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

/// D7 — the bearer token's runtime injection point. Read from the
/// operator's own environment (`BRAIN_SHELL_TOKEN`); held in JS memory
/// only (see src/lib/api/token.ts); never logged, never rendered, never
/// persisted (keyring storage is the recorded M2 deferral). This is
/// system integration (an env read), not business logic.
///
/// Unchanged by Phase I, and that is a decision rather than an omission: the
/// core reads NO token of its own to authorize recall, because the loopback
/// posture needs none (a kernel with no token resolved serves unauthenticated)
/// and a core that read one would give the bearer a second home. The caller's
/// principal rides the webview's own middleware; `RecallQuery` has no field
/// through which one could arrive.
#[tauri::command]
fn kernel_token() -> Option<String> {
    std::env::var("BRAIN_SHELL_TOKEN").ok()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            kernel_token,
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
