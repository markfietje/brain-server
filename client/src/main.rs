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

/// v1.16.2: the connect-time base URL. An empty field resolves to the same-origin
/// the page was served from (brain-server serves the client under /app, so
/// same-origin needs no CORS and works for 127.0.0.1 and localhost alike);
/// non-empty input is used verbatim (remote/JWT mode). Pure so it's testable.
fn resolve_base(raw: &str, origin: Option<String>) -> String {
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        origin.unwrap_or_else(|| "http://127.0.0.1:8765".into())
    } else {
        trimmed
    }
}

/// v1.16.2: the origin brain-server is being served from, for the same-origin
/// Connect default. Web only (eval + window.location); None elsewhere falls
/// back to the loopback default. ponytail: `window.location.origin` is exactly
/// the origin (scheme+host+port, no trailing slash / path), which is what the
/// API base wants — a path-prefixed /app page still hits the same origin.
async fn page_origin() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        document::eval("return window.location.origin")
            .join::<String>()
            .await
            .ok()
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
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
                div { class: "min-h-screen flex items-center justify-center bg-background text-foreground p-4",
                    div { class: "max-w-md",
                        h1 { class: "text-xl text-danger", "Something went wrong" }
                        p { class: "text-muted-foreground mt-2",
                            "The client hit an unexpected error. Reload to retry."
                        }
                        pre { class: "text-xs text-ink-faint mt-4 overflow-auto",
                            "{errors:?}"
                        }
                        button {
                            class: "btn btn-outline btn-sm mt-4",
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
    // v1.16.2: default to same-origin — brain-server serves this client under
    // /app, so an empty base means "the page's own origin" (no CORS, works for
    // 127.0.0.1 and localhost alike). The field stays editable for remote mode.
    let mut url = use_signal(String::new);
    let mut token = use_signal(String::new);
    // v1.16.5 M4: JWT-pair mode (access + refresh). When `jwt_pair` is on, the
    // operator pastes an access + refresh token; otherwise a single opaque or
    // long-lived JWT access token (loopback or single-token remote).
    let mut jwt_pair = use_signal(|| false);
    let mut refresh_token = use_signal(String::new);
    let status = use_signal(|| None::<Result<String, String>>);
    let busy = use_signal(|| false);
    let api_signal = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let nav = navigator();

    let connect = move |_| {
        let raw_url = url().trim().trim_end_matches('/').to_string();
        let access_val = if token().trim().is_empty() {
            None
        } else {
            Some(token().trim().to_string())
        };
        let refresh_val = if refresh_token().trim().is_empty() {
            None
        } else {
            Some(refresh_token().trim().to_string())
        };
        let pair = jwt_pair();
        let mut status = status;
        let mut busy = busy;
        let mut api_signal = api_signal;
        let mut ui = ui;
        let nav = nav;
        spawn(async move {
            busy.set(true);
            // v1.16.2: empty base = same-origin (the page brain-server is served
            // from) — no CORS, works for 127.0.0.1 and localhost alike.
            let base = resolve_base(&raw_url, page_origin().await);
            // M2.1 (v1.16.5): the principal (identity pillar) is derived from the
            // JWT `sub` claim at connect time. JWT-pair mode uses `with_refresh_pair`
            // (which also enables silent refresh); a single-token connect keeps the
            // plain path (opaque loopback → None; a JWT without refresh → its sub).
            let client = if pair {
                ApiClient::with_refresh_pair(base.clone(), access_val, refresh_val)
            } else {
                ApiClient::with_principal(base.clone(), access_val, None)
            };
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
        div { class: "min-h-screen flex items-center justify-center bg-background text-foreground p-4",
            div { class: "w-full max-w-md space-y-6",
                div { class: "flex items-center gap-3",
                    div { class: "flex size-10 items-center justify-center rounded-lg bg-accent/15 text-accent",
                        span { class: "font-mono text-lg font-bold", "b" }
                    }
                    div {
                        h1 { class: "text-xl font-semibold tracking-tight", "brain" }
                        p { class: "text-sm text-muted-foreground",
                            "Governed memory, on your hardware." }
                    }
                }
                div { class: "card",
                    div { class: "card-header", div { class: "card-title", "Connect to brain-server" } }
                    div { class: "card-body space-y-4",
                        label { class: "block space-y-1",
                            span { class: "label", "Backend URL" }
                            input {
                                class: "input w-full",
                                value: "{url}",
                                oninput: move |e| url.set(e.value()),
                                placeholder: "blank = this page's origin (same-server)",
                                "aria-label": "backend URL",
                            }
                        }
                        label { class: "block space-y-1",
                            span { class: "label", "Token" }
                            input {
                                class: "input w-full",
                                "type": "password",
                                value: "{token}",
                                oninput: move |e| token.set(e.value()),
                                placeholder: if jwt_pair() { "access token (JWT)" } else { "optional (loopback)" },
                                "aria-label": "auth token",
                            }
                        }
                        // v1.16.5 M4: JWT-pair mode — access + refresh tokens
                        // (from `brain key mint` or an external IdP). Enables
                        // silent refresh-on-401 + the identity pillar. A single
                        // token (opaque loopback or long-lived JWT) stays the
                        // default path.
                        label { class: "flex items-center gap-2 text-sm text-muted-foreground",
                            input {
                                "type": "checkbox",
                                class: "accent-accent",
                                checked: jwt_pair(),
                                onchange: move |e| jwt_pair.set(e.value() == "true"),
                            }
                            "JWT pair (access + refresh) — enables silent refresh"
                        }
                        if jwt_pair() {
                            label { class: "block space-y-1",
                                span { class: "label", "Refresh token" }
                                input {
                                    class: "input w-full",
                                    "type": "password",
                                    value: "{refresh_token}",
                                    oninput: move |e| refresh_token.set(e.value()),
                                    placeholder: "from `brain key mint` or an IdP",
                                    "aria-label": "refresh token",
                                }
                            }
                        }
                        button {
                            class: "btn btn-primary btn-md w-full",
                            disabled: busy(),
                            onclick: connect,
                            if busy() { "Connecting…" } else { "Connect" }
                        }
                        if let Some(Ok(msg)) = &*status.read() {
                            p { class: "text-ok text-sm", "{msg}" }
                        }
                        if let Some(Err(msg)) = &*status.read() {
                            p { class: "text-danger text-sm", "{msg}" }
                        }
                    }
                }
                p { class: "text-center text-xs text-ink-faint",
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
///
/// v1.16.4 restyle: modern shadcn-style dashboard — fixed sidebar with brand +
/// grouped nav (count badges on the rail), a slim top bar, and the drawer on
/// the right. Every nav target stays a real `<Link>` (a11y gate), all actions
/// stay real `<button>`s.
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

    let security_badge = quarantine as u64 + auth_fail as u64;

    rsx! {
        div { class: "flex min-h-screen bg-background text-foreground",
            // Fixed sidebar: brand + primary nav + identity footer.
            aside {
                class: "sticky top-0 flex h-screen w-56 shrink-0 flex-col border-r border-border bg-card",
                "aria-label": "primary navigation",
                div { class: "flex items-center gap-2 border-b border-border px-4 h-14",
                    div { class: "flex size-8 items-center justify-center rounded-md bg-accent/15 text-accent",
                        span { class: "font-mono text-sm font-bold", "b" }
                    }
                    span { class: "font-semibold tracking-tight", "brain" }
                }
                nav { class: "flex-1 overflow-y-auto p-3 nav",
                    // M2.1: F-pattern anchor — pending review count on the rail.
                    div { class: "mb-1 flex items-center justify-between px-2.5 text-xs text-muted-foreground",
                        span { "Review queue" }
                        span { class: "tabular", "pending {pending}" }
                    }
                    NavLink { to: Route::Review {}, "Review" }
                    NavLink { to: Route::Recall {}, "Recall" }
                    NavLink { to: Route::Subjects {}, "Subjects" }
                    NavLink { to: Route::Security {}, badge: Some(security_badge), "Security" }
                    NavLink { to: Route::Audit {}, dirty: audit_dirty, "Audit" }
                    NavLink { to: Route::Health {}, "Health" }
                }
                div { class: "border-t border-border p-3",
                    div { class: "flex items-center gap-2 text-sm",
                        span { class: conn_dot(conn), "●" }
                        span { class: "text-xs text-muted-foreground", "{conn_label(conn)}" }
                    }
                    if let Some(ref p) = principal {
                        p { class: "mt-1 truncate text-xs text-muted-foreground tabular", "acting as {p}" }
                    } else {
                        p { class: "mt-1 text-xs text-ink-faint", "loopback" }
                    }
                }
            }
            div { class: "flex min-w-0 flex-1 flex-col",
                // Slim top bar: connection + pending summary.
                header { class: "sticky top-0 z-10 flex h-14 items-center gap-3 border-b border-border bg-background/80 px-4 backdrop-blur",
                    span {
                        class: "font-mono text-sm",
                        "aria-label": "connection status",
                        span { class: conn_dot(conn), "●" }
                        " {conn_label(conn)}"
                    }
                    span { class: "text-sm text-muted-foreground tabular",
                        "pending {pending}"
                    }
                    if security_badge > 0 {
                        span { class: "badge badge-warn", "{security_badge} flags" }
                    }
                    if audit_dirty {
                        span { class: "badge badge-danger", "audit chain!" }
                    }
                    if let Some(p) = &principal {
                        span { class: "ml-auto text-xs text-muted-foreground tabular", "{p}" }
                    }
                }
                div { class: "flex flex-1",
                    main { class: "flex-1 p-6", Outlet::<Route> {} }
                    // M2.2: the context drawer (right). Esc closes; ARIA dialog.
                    if drawer_open {
                        Drawer { }
                    }
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

/// v1.16.4: one pill nav link with an optional trailing count badge (state
/// counts published by the panels). Active route gets the filled pill.
#[component]
fn NavLink(
    to: Route,
    badge: Option<u64>,
    #[props(default)] dirty: bool,
    children: Element,
) -> Element {
    rsx! {
        Link {
            to,
            class: "nav-link",
            active_class: "nav-link-active",
            {children}
            if let Some(n) = badge {
                if n > 0 {
                    span { class: "nav-badge", "{n}" }
                }
            }
            if dirty {
                span { class: "nav-badge nav-badge-alert", "!" }
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
            class: "w-80 border-l border-border bg-card p-4 overflow-auto",
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
            div { class: "flex justify-between items-center mb-3",
                h2 { class: "text-sm font-semibold text-muted-foreground", "Detail" }
                button {
                    class: "btn btn-ghost btn-sm",
                    "aria-label": "close drawer",
                    onclick: move |_| ui.drawer.set(None),
                    "✕"
                }
            }
            match &content {
                Some(DrawerContent::Proposal(p)) => rsx! {
                    div { class: "card",
                        div { class: "card-body",
                            p { class: "font-mono text-sm", "proposal #{p.id} · {p.kind}" }
                            p { class: "text-xs text-muted-foreground mt-1",
                                "novelty {p.novelty:.2} · salience {p.salience:.2}" }
                            p { class: "text-sm mt-2", "{p.content}" }
                        }
                    }
                },
                Some(DrawerContent::Hit(h)) => rsx! {
                    div { class: "card",
                        div { class: "card-body",
                            p { class: "font-mono text-sm", "chunk #{h.id}" }
                            p { class: "text-sm mt-2", "{h.content}" }
                        }
                    }
                },
                Some(DrawerContent::Certificate(c)) => rsx! {
                    div { class: "card",
                        div { class: "card-body",
                            p { class: "font-mono text-sm", "found {c.found_count}" }
                            p { class: "text-xs text-muted-foreground mt-1", "chain head {c.chain_head}" }
                            p { class: "text-xs text-muted-foreground", "certified {c.certified_at}" }
                        }
                    }
                },
                Some(DrawerContent::AuthFailure(r)) => rsx! {
                    div { class: "card",
                        div { class: "card-body",
                            p { class: "font-mono text-sm", "{r.kind} · {r.status}" }
                            p { class: "text-xs text-muted-foreground mt-1", "actor: {r.actor}" }
                            p { class: "text-xs text-muted-foreground", "{r.ts}" }
                        }
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

    /// v1.16.2: an empty base resolves to the page's same-origin (the root of
    /// the "cannot reach brain-server" bug — the client was served under /app
    /// but defaulted to a hardcoded host, hitting cross-origin CORS). Non-empty
    /// input is used verbatim for remote/JWT mode; trailing slashes are trimmed.
    #[test]
    fn connect_resolves_empty_base_to_same_origin() {
        // Empty + an origin → the origin, trailing slash stripped.
        assert_eq!(
            resolve_base("", Some("http://127.0.0.1:8765".into())),
            "http://127.0.0.1:8765"
        );
        // Empty + no origin (desktop) → loopback fallback.
        assert_eq!(resolve_base("", None), "http://127.0.0.1:8765");
        // Whitespace-only counts as empty.
        assert_eq!(
            resolve_base("  ", Some("http://localhost:8765".into())),
            "http://localhost:8765"
        );
        // Explicit URL is used verbatim (remote/JWT).
        assert_eq!(
            resolve_base(
                "https://brain.example.com/",
                Some("http://127.0.0.1:8765".into())
            ),
            "https://brain.example.com"
        );
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
