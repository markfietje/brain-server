// The shell's Rust core: system integration ONLY — zero business logic
// (the kernel owns every rule; the webview talks the typed wire). Phase A
// carries exactly ONE command: the D7 token pass-through, which reads the
// OPERATOR'S OWN environment at runtime so the token never enters the JS
// bundle. No fs, no shell, no http, no process plugins; NO unsafe.
#![warn(clippy::all)]
#![deny(clippy::cargo)]
#![forbid(unsafe_code)]

/// D7 — the bearer token's runtime injection point. Read from the
/// operator's own environment (`BRAIN_SHELL_TOKEN`); held in JS memory
/// only (see src/lib/api/token.ts); never logged, never rendered, never
/// persisted (keyring storage is the recorded M2 deferral). This is
/// system integration (an env read), not business logic.
#[tauri::command]
fn kernel_token() -> Option<String> {
    std::env::var("BRAIN_SHELL_TOKEN").ok()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![kernel_token])
        .run(tauri::generate_context!())
        .expect("error while running the shell");
}
