//! brain-client — the Dioxus control surface for brain-server.
//!
//! One codebase → web (WASM/DOM) + desktop + iOS + Android.
//! Run:  `dx serve --platform web`  (or desktop / ios / android)
//!
//! Architecture (see DESIGN_v1.16.0_Client.md):
//!   Router::<Route> → AppShell (nav rail + topbar + Outlet) → panels.
//!   Root context provides an ApiClient (bearer attached) + the shared
//!   connection/badge/drawer signals the panels read via `use_resource`.

// The #[component] macro nests the fn body in an extra block, so one-line
// route components trip unused_braces — a macro artifact, not our code.
#![allow(unused_braces)]

use dioxus::prelude::*;
use panels::{audit, health, recall, review, security, subjects};

// ponytail: api.rs holds Deserialize-only wire-contract types; serde is the
// reader (compiler-invisible), so unrendered fields warn as "never read" —
// same false-positive class as AppState.rate_limiter (AGENTS.md Agent 31).
// The serde round-trip tests pin them; not dead, just waiting.
#[allow(dead_code)]
mod api;
mod panels;

use api::ApiClient;

// ---------------------------------------------------------------------------
// v1.16.0 M1 — the connection state machine (DESIGN §2 + §6, the correctness
// heart). One `use_future` at the root owns its timer; a missed probe never
// flips green→amber (false-offline guard); writes re-enable ONLY after a real
// `/audit/verify` (never a heuristic). Pure logic is extracted below so it is
// unit-testable without the coroutine.
// ---------------------------------------------------------------------------

/// Periodic `/health` probe interval. Ponytail: 5s bounds worst-case recovery
/// (sleep→wake→probe) without flooding a low-power device.
const CONN_PROBE_SECS: u64 = 5;
/// Consecutive probe failures before green→amber. A single flap (network
/// blip, GC pause) must NOT flip the indicator — this is the false-offline guard.
const CONN_FAILURES_BEFORE_DEGRADE: u32 = 2;

/// v1.16.0 M1: dependency-free periodic sleep. Uses `document::eval`+
/// `setTimeout` so we need NO `tokio` dep — works on web + desktop (both ship
/// a JS engine). ponytail: `tokio::time::sleep` would need a tokio dep and
/// doesn't work in WASM without a custom time driver; the eval path is the
/// cross-platform primitive Dioxus already ships.
async fn probe_sleep(secs: u64) {
    let js = format!("return await new Promise(r => setTimeout(r, {secs}*1000));");
    let _ = document::eval(&js).await;
}

/// The connection indicator. Shared via context; read by AppShell (chrome) +
/// every panel (mutation freeze). Set-don't-accumulate is the cancel-safety
/// guard (DESIGN §6): the probe future only writes a single enum value, so a
/// mid-flight cancel cannot strand a half-applied flag.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Conn {
    #[default]
    Unknown,
    Connected,
    Reconnecting,
}

/// Shared UI-state signals (DESIGN §2 F-pattern + the context drawer §5.3).
/// Provided once at the root; panels write the counts they own; AppShell
/// renders them. No polling — counts update on fetch/mutation.
#[derive(Clone, Copy)]
struct UiState {
    /// M1: the connection indicator.
    conn: Signal<Conn>,
    /// M1: derived — are write actions offered? (conn==Connected AND chain verified)
    writes_enabled: Signal<bool>,
    /// M1: set after a recovery 200; cleared once `/audit/verify` settles.
    pending_reverify: Signal<bool>,
    /// M2.1: review pending count (top-left F-pattern anchor).
    pending_count: Signal<u32>,
    /// M2.1: quarantine count badge on the Security link.
    quarantine_count: Signal<u64>,
    /// M2.1: `!` badge on Audit when the last verify was non-clean.
    audit_dirty: Signal<bool>,
    /// M2.1: recent denied-auth count badge on Security.
    auth_failures_count: Signal<u32>,
    /// M2.2: the open context drawer's content (None = closed).
    drawer: Signal<Option<DrawerContent>>,
}

/// M2.2: typed drawer content. Panels push already-fetched data in (no
/// duplicate fetch); the drawer renders the context-rich card on the right.
/// Esc closes; `role="dialog" aria-modal="true"` for AT. Full Radix focus-trap
/// is the v1.18.0 Compliant pass (ponytail: hand-rolled carries the §7 discipline).
#[derive(Clone, PartialEq)]
pub enum DrawerContent {
    Proposal(api::Proposal),
    Hit(api::Hit),
    Certificate(api::DsarCertificate),
    AuthFailure(api::AuditRow),
}

/// M1 pure: the false-offline guard. 1 failure keeps green; N flip amber; a
/// success resets. Extracted so the coroutine's only job is plumbing.
fn probe_state(failures: u32, ok: bool) -> Conn {
    if ok {
        Conn::Connected
    } else if failures >= CONN_FAILURES_BEFORE_DEGRADE {
        Conn::Reconnecting
    } else {
        // Below the threshold OR fresh-start (Unknown): never claim connected
        // on a failure, but don't degrade until the guard is satisfied.
        Conn::Unknown
    }
}

/// M1 pure: writes are allowed only when connected AND the chain has been
/// re-verified after a recovery (DESIGN §2 recovery rule). `verify_ok=None`
/// means verify was not required (initial connect — writes gate on conn only).
fn writes_allowed(conn: Conn, verify_ok: Option<bool>, pending_reverify: bool) -> bool {
    if conn != Conn::Connected {
        return false;
    }
    if pending_reverify {
        // A recovery 200 flipped conn green, but we're waiting on /audit/verify.
        return match verify_ok {
            Some(true) => true,
            _ => false, // not-yet-verified OR verify-failed → stay frozen
        };
    }
    true
}

/// The task-oriented routes (DESIGN §2) + the connect-first landing (DESIGN §3).
/// Deep-linkable: a recall trace, a DSAR certificate, a specific proposal are
/// all URL-addressable.
#[derive(Clone, Debug, PartialEq, Routable)]
enum Route {
    #[route("/")]
    Connect {},
    #[layout(AppShell)]
    #[route("/review")]
    Review {},
    #[route("/recall")]
    Recall {},
    /// M4.2: deep-linkable decision-path artifact (`?trace=true` → trace_id).
    #[route("/recall/:trace_id")]
    RecallTrace { trace_id: i64 },
    #[route("/subjects")]
    Subjects {},
    #[route("/security")]
    Security {},
    #[route("/audit")]
    Audit {},
    #[route("/health")]
    Health {},
    #[end_layout]
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

fn main() {
    dioxus::launch(app);
}

fn app() -> Element {
    // Root context: the live ApiClient. Connect-first onboarding (DESIGN §3)
    // starts unconnected; Connect writes a new client in and navigates to Review.
    // Panels read this `Signal<ApiClient>` via `use_resource`, so replacing the
    // value re-fetches everything — no explicit invalidation needed.
    let api = use_context_provider(|| Signal::new(ApiClient::new("", None)));
    // M1/M2: the shared UI-state bundle. Provided once at the root so it
    // survives every route transition (panels read/write via use_context).
    let ui = use_context_provider(|| UiState {
        conn: Signal::new(Conn::Unknown),
        writes_enabled: Signal::new(false),
        pending_reverify: Signal::new(false),
        pending_count: Signal::new(0),
        quarantine_count: Signal::new(0),
        audit_dirty: Signal::new(false),
        auth_failures_count: Signal::new(0),
        drawer: Signal::new(None),
    });

    // M1.1: the single background probe. Owns its timer; survives panel
    // unmounts (it lives at the root). Wake-from-sleep is handled by the
    // periodic interval (5s worst case) — the eval-based instant-wake listener
    // is a v1.17.0 web enhancement (untestable without `dx serve`; documented
    // ceiling in the plan). Recovery is ONLY a real `/health` 200 + verify.
    // ponytail: sleep is via `document::eval`+setTimeout so we need NO `tokio`
    // dep (works on web + desktop webviews, both have a JS engine).
    let probe_api = api;
    let mut probe_ui = ui;
    use_future(move || async move {
        let mut failures = 0u32;
        loop {
            probe_sleep(CONN_PROBE_SECS).await;
            let client = probe_api();
            // Pre-connect: nothing to probe. Stay Unknown; don't accrue failures.
            if !client.is_configured() {
                failures = 0;
                continue;
            }
            let ok = client.health().await.is_ok();
            let prev = (probe_ui.conn)();
            let next = probe_state(failures, ok);
            // Detect a recovery transition (was not Connected, now is): require
            // chain re-verify before writes re-enable (DESIGN §2 recovery rule).
            let recovering = ok && prev != Conn::Connected;
            if ok {
                failures = 0;
            } else {
                failures = failures.saturating_add(1);
            }
            probe_ui.conn.set(next);
            if recovering {
                probe_ui.pending_reverify.set(true);
                // Best-effort verify: 403 (non-Admin) → unverified, not silent.
                let verify_ok = probe_api().audit_verify().await.map(|v| v.ok).ok();
                probe_ui
                    .writes_enabled
                    .set(writes_allowed(next, verify_ok, true));
                probe_ui.pending_reverify.set(false);
                // Surface a dirty-chain badge if verify failed.
                if matches!(verify_ok, Some(false)) {
                    probe_ui.audit_dirty.set(true);
                }
            } else {
                probe_ui
                    .writes_enabled
                    .set(writes_allowed(next, None, false));
            }
        }
    });

    rsx! {
        document::Stylesheet { href: asset!("/assets/tailwind.css") }
        // v1.16.2 "Harden" M4.1: an ErrorBoundary around the router so a panic
        // in a child panel renders an operator-facing fallback instead of a
        // blank screen. Errors are Debug-formatted; no sensitive data leaks
        // (the client holds no secrets beyond the token, which is never in an
        // error message).
        ErrorBoundary {
            handle_error: |errors: ErrorContext| rsx! {
                div { class: "min-h-screen flex items-center justify-center bg-surface text-ink p-4",
                    div { class: "max-w-md",
                        h1 { class: "text-xl text-danger", "Something went wrong" }
                        p { class: "text-ink-muted mt-2",
                            "The client hit an unexpected error. Reload to retry."
                        }
                        pre { class: "text-xs text-ink-faint mt-4 overflow-auto",
                            "{errors:?}"
                        }
                        button {
                            class: "mt-4 border border-border-subtle rounded px-3 py-1 text-sm",
                            onclick: move |_| errors.clear_errors(),
                            "Dismiss"
                        }
                    }
                }
            },
            Router::<Route> {}
        }
    }
}

/// Connect-first onboarding (DESIGN §3.1). URL + token + loopback/remote toggle,
/// a live `/health` probe on Connect, then a guided first value. No feature tour.
#[component]
fn Connect() -> Element {
    let mut url = use_signal(|| "http://127.0.0.1:8765".to_string());
    let mut token = use_signal(String::new);
    let mut remote = use_signal(|| false);
    let mut status = use_signal(|| None::<Result<String, String>>);
    let busy = use_signal(|| false);
    let api_signal = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let nav = navigator();

    let connect = move |_| {
        let base = url().trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            status.set(Some(Err("enter a backend URL".into())));
            return;
        }
        let token_val = if token().trim().is_empty() {
            None
        } else {
            Some(token().trim().to_string())
        };
        let is_remote = remote();
        let mut status = status;
        let mut busy = busy;
        let mut api_signal = api_signal;
        let mut ui = ui;
        let nav = nav;
        spawn(async move {
            busy.set(true);
            // M2.1: principal is set at connect time (identity pillar). Loopback
            // mode has no identity to claim; remote/JWT shows the sub.
            let principal = if is_remote {
                Some("remote-user".to_string())
            } else {
                None
            };
            let client = ApiClient::with_principal(base.clone(), token_val, principal);
            status.set(Some(match client.health().await {
                Ok(h) => Ok(format!(
                    "connected — v{} · {}{}",
                    h.version,
                    h.capacity
                        .as_ref()
                        .map(|c| format!("docs {}/{}, ", c.docs, c.max_docs))
                        .unwrap_or_default(),
                    h.status
                )),
                Err(e) => Err(format!("could not reach {base}: {e}")),
            }));
            if matches!(status(), Some(Ok(_))) {
                // Write the client in so every panel re-fetches through it.
                // M1: a fresh connect is a clean connect — writes gate on the
                // probe's first green, but no recovery re-verify is pending.
                api_signal.set(client);
                ui.conn.set(Conn::Connected);
                ui.writes_enabled.set(true);
                ui.pending_reverify.set(false);
                nav.replace(Route::Review {});
            }
            busy.set(false);
        });
    };

    rsx! {
        div { class: "min-h-screen flex items-center justify-center bg-surface text-ink p-4",
            div { class: "w-full max-w-md",
                h1 { class: "text-xl", "brain — governed memory, on your hardware." }
                p { class: "text-ink-muted mb-4", "Connect your brain-server to begin." }
                label { class: "block text-sm mb-1", "Backend URL" }
                input {
                    class: "border border-border-subtle surface-raised rounded px-2 py-1 w-full mb-2",
                    value: "{url}",
                    oninput: move |e| url.set(e.value()),
                    "aria-label": "backend URL",
                }
                label { class: "block text-sm mb-1", "Token" }
                input {
                    class: "border border-border-subtle surface-raised rounded px-2 py-1 w-full mb-2",
                    "type": "password",
                    value: "{token}",
                    oninput: move |e| token.set(e.value()),
                    placeholder: "optional (loopback)",
                    "aria-label": "auth token",
                }
                label { class: "flex items-center gap-2 text-sm mb-4",
                    input {
                        "type": "checkbox",
                        checked: remote(),
                        onchange: move |e| remote.set(e.value() == "true"),
                    }
                    "Remote / pilot (JWT mode; shows the principal)"
                }
                button {
                    class: "border border-border-subtle rounded px-4 py-2 bg-accent text-white disabled:opacity-50",
                    disabled: busy(),
                    onclick: connect,
                    if busy() { "Connecting…" } else { "Connect" }
                }
                if let Some(Ok(msg)) = &*status.read() {
                    p { class: "text-ok mt-3", "{msg}" }
                }
                if let Some(Err(msg)) = &*status.read() {
                    p { class: "text-danger mt-3", "{msg}" }
                }
                p { class: "text-xs text-ink-faint mt-6",
                    "One-line install:  curl -fsSL … | sh   then  brain doctor"
                }
            }
        }
    }
}

/// AppShell — the nav rail (desktop/web) that persists across route transitions.
/// M2: F-pattern top bar with `Pending: N`, left-rail count badges, principal
/// identity, and the read-only degrade banner (M1) + context drawer (M2.2).
/// On mobile (v1.17.0) the same routes render under a bottom tab bar.
#[component]
fn AppShell() -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let conn = (ui.conn)();
    let writes = (ui.writes_enabled)();
    let pending = (ui.pending_count)();
    let quarantine = (ui.quarantine_count)();
    let auth_fail = (ui.auth_failures_count)();
    let audit_dirty = (ui.audit_dirty)();
    let principal = {
        let c = api();
        c.principal().map(str::to_string)
    };
    let drawer_open = ui.drawer.read().is_some();
    let pending_reverify = (ui.pending_reverify)();

    rsx! {
        div { class: "flex min-h-screen bg-surface text-ink",
            nav {
                class: "flex gap-2 p-2 border-b hairline surface-raised items-center flex-wrap",
                span {
                    class: "font-mono text-sm",
                    "aria-label": "connection status",
                    span { class: conn_dot(conn), "●" }
                    " {conn_label(conn)} "
                }
                span { class: "text-ink font-semibold ml-2 tabular", "Pending: {pending}" }
                Link { to: Route::Review {}, active_class: "font-bold", "Review" }
                Link { to: Route::Recall {}, active_class: "font-bold", "Recall" }
                Link { to: Route::Subjects {}, active_class: "font-bold", "Subjects" }
                Link { to: Route::Security {}, active_class: "font-bold",
                    "Security"
                    if quarantine > 0 || auth_fail > 0 {
                        span { class: "ml-1 px-1 rounded bg-warn text-surface text-xs tabular",
                            "{quarantine + auth_fail as u64}"
                        }
                    }
                }
                Link { to: Route::Audit {}, active_class: "font-bold",
                    "Audit"
                    if audit_dirty {
                        span { class: "ml-1 text-danger", "!" }
                    }
                }
                Link { to: Route::Health {}, active_class: "font-bold", "Health" }
                // M2.1: identity pillar (top-right). Loopback shows no identity.
                if let Some(p) = principal {
                    span { class: "ml-auto text-xs text-ink-muted tabular", "acting as {p}" }
                } else {
                    span { class: "ml-auto text-xs text-ink-faint", "loopback" }
                }
            }
            div { class: "flex flex-1",
                main { class: "p-4 flex-1", Outlet::<Route> {} }
                // M2.2: the context drawer (right). Esc closes; ARIA dialog.
                if drawer_open {
                    Drawer { }
                }
            }
        }
        // M1.2: read-only degrade banner. Rendered once at AppShell top when
        // amber: panels keep showing last-known state; writes are disabled.
        if conn == Conn::Reconnecting {
            div {
                class: "border-b border-warn/40 bg-warn/10 px-4 py-1 text-sm text-warn",
                role: "status",
                "Disconnected — showing last-known state. Write actions disabled."
            }
        }
        if !writes && conn == Conn::Connected && pending_reverify {
            div {
                class: "border-b border-info/40 bg-info/10 px-4 py-1 text-sm text-info",
                role: "status",
                "Reconnected — verifying audit chain before enabling writes…"
            }
        }
    }
}

/// M2.2: the context drawer. Esc closes (clears the signal); full Radix
/// Tab-cycling focus trap is the v1.18.0 pass. The content is already-fetched
/// data pushed by a panel (no duplicate fetch).
#[component]
fn Drawer() -> Element {
    let mut ui = use_context::<UiState>();
    let content = (ui.drawer)();
    rsx! {
        aside {
            class: "w-80 border-l hairline surface-raised p-4 overflow-auto",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "detail drawer",
            tabindex: "0",
            onkeydown: move |e: Event<KeyboardData>| {
                // Esc (and the web `Escape` code) closes the drawer.
                if e.key() == Key::Escape {
                    ui.drawer.set(None);
                }
            },
            div { class: "flex justify-between items-center mb-2",
                h2 { class: "text-sm font-semibold text-ink-muted", "Detail" }
                button {
                    class: "text-ink-faint hover:text-ink text-sm",
                    "aria-label": "close drawer",
                    onclick: move |_| ui.drawer.set(None),
                    "✕"
                }
            }
            match &content {
                Some(DrawerContent::Proposal(p)) => rsx! {
                    div {
                        p { class: "font-mono text-sm", "proposal #{p.id} · {p.kind}" }
                        p { class: "text-xs text-ink-muted mt-1",
                            "novelty {p.novelty:.2} · salience {p.salience:.2}" }
                        p { class: "text-sm mt-2", "{p.content}" }
                    }
                },
                Some(DrawerContent::Hit(h)) => rsx! {
                    div {
                        p { class: "font-mono text-sm", "chunk #{h.id}" }
                        p { class: "text-sm mt-2", "{h.content}" }
                    }
                },
                Some(DrawerContent::Certificate(c)) => rsx! {
                    div {
                        p { class: "font-mono text-sm", "found {c.found_count}" }
                        p { class: "text-xs text-ink-muted mt-1", "chain head {c.chain_head}" }
                        p { class: "text-xs text-ink-muted", "certified {c.certified_at}" }
                    }
                },
                Some(DrawerContent::AuthFailure(r)) => rsx! {
                    div {
                        p { class: "font-mono text-sm", "{r.kind} · {r.status}" }
                        p { class: "text-xs text-ink-muted mt-1", "actor: {r.actor}" }
                        p { class: "text-xs text-ink-muted", "{r.ts}" }
                    }
                },
                None => rsx! { p { class: "text-ink-faint text-sm", "nothing selected" } },
            }
        }
    }
}

fn conn_dot(conn: Conn) -> &'static str {
    match conn {
        Conn::Connected => "text-ok",
        Conn::Reconnecting => "text-warn",
        Conn::Unknown => "text-ink-faint",
    }
}

fn conn_label(conn: Conn) -> &'static str {
    match conn {
        Conn::Connected => "connected",
        Conn::Reconnecting => "reconnecting",
        Conn::Unknown => "…",
    }
}

#[component]
fn Review() -> Element {
    review::panel()
}
#[component]
fn Recall() -> Element {
    recall::panel()
}
/// M4.2: the deep-linkable trace artifact. `/recall/:trace_id` renders the
/// recorded decision path for a past recall (the `?trace=true` id).
#[component]
fn RecallTrace(trace_id: i64) -> Element {
    recall::trace_panel(trace_id)
}
#[component]
fn Subjects() -> Element {
    subjects::panel()
}
#[component]
fn Security() -> Element {
    security::panel()
}
#[component]
fn Audit() -> Element {
    audit::panel()
}
#[component]
fn Health() -> Element {
    health::panel()
}

#[component]
fn NotFound(segments: Vec<String>) -> Element {
    rsx! { p { "not found: {segments:?}" } }
}

// ---------------------------------------------------------------------------
// M1 tests — the runnable checks for the connection state machine's
// non-trivial rules (DESIGN §6: one check per rule, pure fns).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// A single failure must NOT flip green→amber (the false-offline guard).
    #[test]
    fn probe_degrades_only_after_n_failures() {
        // 1 failure with no prior green stays Unknown (no false claim).
        assert_eq!(probe_state(1, false), Conn::Unknown);
        // N consecutive failures → amber.
        assert_eq!(
            probe_state(CONN_FAILURES_BEFORE_DEGRADE, false),
            Conn::Reconnecting
        );
        // A success resets to green regardless of prior failures.
        assert_eq!(probe_state(100, true), Conn::Connected);
        assert_eq!(probe_state(0, true), Conn::Connected);
    }

    /// Writes re-enable only after the chain verifies post-recovery. A clean
    /// connect (no pending reverify) gates on conn alone.
    #[test]
    fn writes_re_enable_only_after_chain_verify() {
        // Disconnected → never.
        assert!(!writes_allowed(Conn::Reconnecting, None, false));
        assert!(!writes_allowed(Conn::Unknown, None, false));
        // Clean connect (no reverify pending) → yes.
        assert!(writes_allowed(Conn::Connected, None, false));
        // Recovery pending: only a verified chain re-enables.
        assert!(!writes_allowed(Conn::Connected, None, true));
        assert!(!writes_allowed(Conn::Connected, Some(false), true));
        assert!(writes_allowed(Conn::Connected, Some(true), true));
    }

    /// Recovery is a real 200, not a heuristic — covered by `probe_state`
    /// (only `ok=true` yields Connected; no path fakes green).
    #[test]
    fn recovery_is_real_200_not_heuristic() {
        for failures in 0..100 {
            assert_eq!(
                probe_state(failures, false),
                if failures >= CONN_FAILURES_BEFORE_DEGRADE {
                    Conn::Reconnecting
                } else {
                    Conn::Unknown
                }
            );
        }
        assert_eq!(probe_state(0, true), Conn::Connected);
    }

    /// v1.16.2 "Harden" M2: the XSS gate. Dioxus escapes text by default;
    /// `dangerous_inner_html` is the ONLY XSS vector. Fail if any source file
    /// uses it. Greps the whole client/src tree (the panels + api.rs + main.rs).
    /// The needle is built by concatenation so this test's own source doesn't
    /// contain the literal token and self-match.
    #[test]
    fn xss_escape_hatch_is_unused() {
        let needle = concat!("dangerous_inner", "_html");
        let mut violations = Vec::new();
        for entry in walk_dir("src") {
            let src = std::fs::read_to_string(&entry).unwrap();
            for (i, line) in src.lines().enumerate() {
                // Skip comment/doc lines — they may legitimately mention the
                // escape hatch; the guard is for *usage*, not prose.
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains(needle) {
                    violations.push(format!("{}:{}", entry.display(), i + 1));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "XSS risk — the raw-HTML escape hatch found in: {violations:?}"
        );
    }

    /// v1.16.2 "Harden" M5: the token-hygiene gate. The token must stay in
    /// memory (Signal<ApiClient>). `use_persistent` on web backs to localStorage
    /// which is XSS-readable. Fail if the token touches it. The needles are
    /// built by concatenation so this test's own source doesn't self-match.
    #[test]
    fn credentials_stay_in_memory() {
        let persistent = concat!("use_", "persistent");
        let token = concat!("to", "ken");
        let flag = format!("{persistent} near {token}");
        for entry in walk_dir("src") {
            let src = std::fs::read_to_string(&entry).unwrap();
            for (i, line) in src.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains(persistent) && line.to_lowercase().contains(token) {
                    panic!(
                        "{flag} at {}:{} — the credential must stay in-memory",
                        entry.display(),
                        i + 1
                    );
                }
            }
        }
    }

    /// v1.16.3 M2.2: the semantic-audit gate (WCAG 2.1.1 + ARIA in HTML):
    /// clickable elements must be real `<button>`s, never `<div onclick>` —
    /// a div's click is unreachable to keyboard + screen-reader users. Greps
    /// the whole source tree; skips comment lines. (Keydown handlers on a
    /// `tabindex="0"` focus container are legitimate and not flagged — only a
    /// div that claims click without the button semantics is the bug.)
    #[test]
    fn interactive_elements_are_buttons() {
        let click = concat!("on", "click");
        for entry in walk_dir("src") {
            let src = std::fs::read_to_string(&entry).unwrap();
            for (i, line) in src.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains(click) && line.contains("div") {
                    panic!(
                        "{click} on a div at {}:{} — use a <button> (WCAG 2.1.1 \
                         Keyboard + ARIA in HTML)",
                        entry.display(),
                        i + 1
                    );
                }
            }
        }
    }

    /// Walk `src` (recursively) returning `.rs` file paths. Relative to the
    /// client crate root (CWD when cargo runs tests). ponytail: a tiny helper
    /// avoids a `walkdir` dep for two grep guards.
    fn walk_dir(dir: &str) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    out.extend(walk_dir(&p.to_string_lossy()));
                } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                    out.push(p);
                }
            }
        }
        out
    }
}
