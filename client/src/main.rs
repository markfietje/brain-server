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
use panels::{
    audit, create, data, graph, health, ops, overview, recall, register, review, security,
    subjects, system, ump,
};

// ponytail: api.rs holds Deserialize-only wire-contract types; serde is the
// reader (compiler-invisible), so unrendered fields warn as "never read" —
// same false-positive class as AppState.rate_limiter (AGENTS.md Agent 31).
// The serde round-trip tests pin them; not dead, just waiting.
#[allow(dead_code)]
mod api;
// v1.28.1 M4: the shared two-step confirm for destructive buttons (purge,
// DSAR erasure, quarantine delete, reindex) — see the module.
mod confirm;
// v1.23.0 M3 "Roles": client-side role capability + panel-visibility helpers
// (defense-in-depth UI only — the server enforces). See the module.
mod role;
// v1.16.8 M1–M5: i18n (t / LOCALE), RTL readiness, theme + density toggles,
// and locale-aware number formatting. Zero-dep FTL parsing — see the module.
mod i18n;
mod panels;
// v1.20.0 M3: the offline-tolerance action queue (queue + replay, not full
// offline) — see the module for the idempotency + persistence rules.
mod queue;
// v1.28.1 M4: the destructive-replay review banner (parked Purge/DSAR
// actions behind an explicit per-item Replay/Skip decision).
mod replay;
// v1.16.6 M2: the secure-token storage seam (OS keyring on non-web, no-op on
// web). save/load (connect + auto-reconnect) and delete (v1.16.7 M7.1 logout)
// each have a caller in every cfg variant, so no `allow(dead_code)`.
mod storage;
// 1.28.4 M1-M3 scaffolds (steward-ip plan): the api_proxy envelope seam, the
// slot registry, and the conversation assembler land ahead of their UI
// consumers — same truthful-allow posture as the workflow substrate (the
// connector-module precedent). Each is `#[cfg(test)]`-covered now; the
// dead-code watchdog re-flags if a scaffold ships without its consumer.
mod api_proxy;
// The approval control center dock — the HITL queue as a
// conversation.input.dock slot entry with digest-bound decisions.
mod approvals;
mod conversation;
mod slots;
// The renderer contract over the slot registry (root + keyed chat
// dispatch + dock order) — consumed by the approval control center (M5).
mod ui_renderer;
// v1.20.15 "Clock": the shared proposal-deadline clock core (tier + format
// math), consumed by the review cards, the detail page, and /ops.
mod time_budget;

use api::ApiClient;
use i18n::{t, t_fmt};

/// v1.20.3 "Classify" (G5): the operator-console render boundary. Stored
/// content may contain invisible Unicode that a server screen let through as
/// `clean` (the blocklist + classifier strip it for *their* decision, but the
/// bytes stay verbatim at rest). This strips those invisible chars from
/// *displayed* content so the operator sees the de-obfuscated form.
///
/// Synchronization contract: this is a DELIBERATE COPY of the server lib's
/// `strip_invisible`/`is_invisible` (src/strip_invisible.rs) — the wasm
/// bundle cannot link the server crate, so the set is mirrored by hand and
/// kept in lockstep in the same change (the codepoints below must match the
/// server's, including the v1.27.14 F-32 additions). The codepoint pins in
/// `strip_invisible_removes_smuggling_but_keeps_visible_text` below are the
/// client-side drift alarm; changing the server set means changing this one
/// in the same PR.
/// Pure + idempotent; the raw bytes are never rewritten at rest.
pub(crate) fn strip_invisible(input: &str) -> String {
    input.chars().filter(|&c| !is_invisible(c)).collect()
}

/// True for a char that renders invisibly and is used to smuggle
/// instruction/exfiltration bytes or defeat substring matching.
fn is_invisible(c: char) -> bool {
    let cp = c as u32;
    // Tag block (U+E0000–E007F), variation selectors (U+FE00–FE0F plus the
    // supplementary range U+E0100–U+E01EF — F-32), zero-width
    // space/non-joiner/joiner + word joiner, the legacy BOM / function +
    // abbreviation + invisible separators / soft hyphen / grapheme joiner, and
    // the Unicode Bidi_Control set (U+200E/200F, U+202A–202E, U+2066–2069,
    // U+061C ALM — F-32 — the Trojan Source / W3C TR#20 directional
    // smuggling class).
    (0xE0000..=0xE007F).contains(&cp)
        || (0xFE00..=0xFE0F).contains(&cp)
        || (0xE0100..=0xE01EF).contains(&cp)
        || (0x200E..=0x200F).contains(&cp)
        || (0x202A..=0x202E).contains(&cp)
        || (0x2066..=0x2069).contains(&cp)
        || cp == 0x061C
        || matches!(cp, 0x200B | 0x200C | 0x200D | 0x2060)
        || matches!(cp, 0xFEFF | 0x2061 | 0x2062 | 0x2063 | 0x00AD | 0x034F)
}

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
pub(crate) async fn probe_sleep(secs: u64) {
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

/// v1.27.20 M2.2 (F-37 audit): the plaintext-http guard — a non-loopback
/// `http://` base (e.g. `http://10.0.0.5:8765`) sends the auth token in
/// cleartext, so Connect warns BEFORE the operator submits. Loopback hosts
/// (127.x, ::1, localhost) are exempt — the dev default must not nag. Pure so
/// it's testable. Degenerate inputs warn only when they are really
/// scheme-http + non-loopback (garbage fails at the request, not here).
fn plaintext_http(url: &str) -> bool {
    let base = resolve_base(url, None);
    let rest = match base.strip_prefix("http://") {
        Some(r) => r,
        None => return false, // empty→loopback default, https, or junk
    };
    let host = if rest.contains('[') {
        // bracketed IPv6 — the port split must not touch the address itself.
        let end = rest.rfind(']').map(|i| i + 1).unwrap_or(rest.len());
        rest[..end].trim_matches(['[', ']'])
    } else if let Some(cut) = rest.rfind(':') {
        // if the last ':' is an address char (colons 0..1), it's a bare
        // unbracketed v6 with no port — keep the whole rest.
        if cut <= 1 && rest[..=cut].ends_with("::") {
            rest
        } else {
            &rest[..cut]
        }
    } else {
        rest
    };
    let host = host.to_ascii_lowercase();
    !(host == "localhost" || host.starts_with("127.") || host == "::1")
}

/// v1.17.0 M3.4: the offline-prefill rule — a remembered base pre-fills the
/// URL field ONLY when the field is empty (never overwrite what the operator
/// typed). Pure so the connect-screen effect is plumbing.
fn prefill_if_empty(current: &str, remembered: &str) -> Option<String> {
    if current.trim().is_empty() {
        Some(remembered.to_string())
    } else {
        None
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
    /// v1.16.7 M5: the command palette's open state.
    palette_open: Signal<bool>,
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
    /// v1.17.6 M2.6: Connect moved to `/connect` (outside the AppShell layout)
    /// so the landing `/` becomes the authenticated Overview home.
    #[route("/connect")]
    Connect {},
    #[layout(AppShell)]
    /// v1.17.6 M2.6: the decision-first landing page.
    #[route("/")]
    Overview {},
    #[route("/review")]
    Review {},
    /// v1.16.7 M1: a deep-linkable specific proposal (share a review card).
    #[route("/review/:proposal_id")]
    ReviewDetail { proposal_id: i64 },
    #[route("/recall")]
    Recall {},
    /// M4.2: deep-linkable decision-path artifact (`?trace=true` → trace_id).
    #[route("/recall/:trace_id")]
    RecallTrace { trace_id: i64 },
    /// v1.17.7 M3: Explore group — browse + traverse the knowledge graph.
    #[route("/graph")]
    Graph {},
    #[route("/subjects")]
    Subjects {},
    /// v1.16.7 M1: a deep-linkable deletion certificate (GDPR Art 17 evidence).
    #[route("/subjects/certificate/:dsar_id")]
    DsarDetail { dsar_id: i64 },
    #[route("/security")]
    Security {},
    /// v1.19.0 M2: `/audit?since=&principal=` — the audit filters are URL-
    /// addressable, so a reviewer can share a filtered audit view.
    #[route("/audit?:since&:principal")]
    Audit {
        since: Option<String>,
        principal: Option<String>,
    },
    /// v1.17.7 M4: Write group — create workspace (ingest/procedures/consolidate).
    #[route("/create")]
    Create {},
    #[route("/health")]
    Health {},
    /// v1.17.8 M5: Rights group — data (purge/export/retention).
    #[route("/data")]
    Data {},
    /// v1.17.8 M6: Portability group — UMP wire ops.
    #[route("/ump")]
    Ump {},
    /// v1.17.8 M7: System group — console (domains/snapshot/art30/reindex/sources/console).
    #[route("/system")]
    System {},
    /// v1.20.6 M1: Memory Operations — the live pending-queue + SLA countdown
    /// clocks + the flagged/quarantined screen output. The HITL work surface.
    #[route("/ops")]
    Ops {},
    /// v1.20.9 M1–M2: Agent Memory Register — read-only provenance ledger.
    #[route("/register")]
    Register {},
    /// v1.27.11 "Console": the BPO dashboard views. `client-auditor` renders
    /// its own single-client dashboard; `bpo-ops`/admin renders the all-clients
    /// board. Role-gated in the shell nav + inside the panel (defense-in-depth;
    /// the server enforces the register rows either way).
    #[route("/clients")]
    Clients {},
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
        palette_open: Signal::new(false),
    });

    // v1.16.8: restore persisted UI prefs (theme / density / locale) on launch.
    // Best-effort (web localStorage; no-op elsewhere); sanitized so a corrupt
    // stored value can never produce an unsupported theme/density/locale.
    //
    // v1.27.21 (N15): the restores ride the async eval seam, i.e. they land
    // AFTER the first render — a de/fr/es/nl operator saw an English first
    // paint. The router is therefore gated behind a locale-neutral shell
    // until this future settles (`booted` below).
    // v1.27.21 (N7): the per-install DSAR salt rides the same restore pass.
    let mut booted = use_signal(|| false);
    use_future(move || async move {
        if let Some(v) = i18n::pref_load("theme").await {
            i18n::theme().set(i18n::pick_theme(&v));
        }
        if let Some(v) = i18n::pref_load("density").await {
            i18n::density().set(i18n::pick_density(&v));
        }
        if let Some(v) = i18n::pref_load("locale").await {
            i18n::locale().set(i18n::pick_locale(&v));
        }
        if let Some(v) = i18n::pref_load(queue::SALT_PREF_KEY).await {
            queue::salt().set(Some(v));
        }
        // v1.20.0 M3: restore any offline-queued actions (web localStorage).
        if let Some(v) = i18n::pref_load(queue::QUEUE_PREF_KEY).await {
            let items = queue::queue_from_json(&v);
            if !items.is_empty() {
                queue::queue().set(items);
            }
        }
        booted.set(true);
    });

    // v1.16.8 M3/M4/M2: apply theme + density + RTL dir to the document root.
    // Each effect reads its signal, so it re-runs when the pref changes (no
    // page reload). `{theme:?}` emits a JS-quoted string literal.
    use_effect(move || {
        let theme = i18n::theme()();
        document::eval(&format!(
            "document.documentElement.dataset.theme = {theme:?}"
        ));
    });
    use_effect(move || {
        let density = i18n::density()();
        document::eval(&format!(
            "document.documentElement.dataset.density = {density:?}"
        ));
    });
    use_effect(move || {
        let locale = i18n::locale()();
        let dir = if i18n::is_rtl(locale) { "rtl" } else { "ltr" };
        document::eval(&format!("document.documentElement.dir = {dir:?}"));
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

    // v1.20.0 M3 — replay the offline queue when the connection returns.
    // The effect subscribes to conn + queue length, so it fires on the
    // amber→green transition and on every queue change. No in-flight guard
    // needed: `take_replayable` drains, so an overlapping run has nothing to
    // take (and a settled action is never re-fired — `replay_applied`).
    //
    // v1.28.1 M4 (F-34): ONLY the non-destructive actions auto-replay.
    // Destructive ones stay parked in the queue and surface in the review
    // banner (`replay.rs`) — an irreversible op never fires without a human.
    //
    // v1.27.21 (N5): a server-ANSWERED failure (not offline, not 404-already-
    // done) costs the action one of its `MAX_REPLAY_RETRIES` — a persistently
    // 500-ing action used to re-enqueue and refire forever. Offline failures
    // stay free: they succeed once the connection is genuinely back.
    let replay_ui = ui;
    use_effect(move || {
        let connected = (replay_ui.conn)() == Conn::Connected;
        let queued = !queue::queue().read().is_empty();
        if !(connected && queued) {
            return;
        }
        let auto = queue::take_replayable();
        if auto.is_empty() {
            return;
        }
        spawn(async move {
            let api = api();
            for action in auto {
                use queue::QueuedAction::*;
                let res = match &action {
                    Approve {
                        id,
                        supersedes,
                        digest,
                        ..
                    } => api
                        .approve_proposal(*id, *supersedes, digest.as_deref())
                        .await
                        .map(|_| ()),
                    Reject { id, .. } => api.reject_proposal(*id, None).await.map(|_| ()),
                    Edit { id, content, .. } => api.edit_proposal(*id, content).await.map(|_| ()),
                    Purge { .. } | Dsar { .. } => Ok(()), // never auto-fired
                };
                if queue::replay_applied(&res) {
                    continue;
                }
                match res {
                    Err(e) if queue::is_offline(&e) => {
                        queue::enqueue(action); // stay queued, no retry cost
                    }
                    _ => queue::requeue_failed(&action), // parks at the cap
                }
            }
        });
    });

    rsx! {
        document::Stylesheet { href: asset!("/assets/tailwind.css") }
        // The premium polish layers (progressive enhancement —
        // both are additive CSS, no first-paint dependency).
        document::Stylesheet { href: asset!("/assets/components-enhanced.css") }
        document::Stylesheet { href: asset!("/assets/animations-enhanced.css") }
        if booted() {
            // v1.16.2 "Harden" M4.1: an ErrorBoundary around the router so a panic
            // in a child panel renders an operator-facing fallback instead of a
            // blank screen. Errors are Debug-formatted; no sensitive data leaks
            // (the client holds no secrets beyond the token, which is never in an
            // error message).
            ErrorBoundary {
                handle_error: |errors: ErrorContext| {
                    // M3.1 (v1.27.20): the crash path speaks the current locale —
                    // the strings resolve fresh because `t` subscribes to locale
                    // changes (the fallback panel is a component, not a closure
                    // capturing one render's translations).
                    rsx! {
                        div { class: "min-h-screen flex items-center justify-center bg-background text-foreground p-4",
                            div { class: "max-w-md",
                                h1 { class: "text-xl text-danger", {t("err_title")} }
                                p { class: "text-muted-foreground mt-2",
                                    {t("err_body")}
                                }
                                pre { class: "text-xs text-ink-faint mt-4 overflow-auto",
                                    "{errors:?}"
                                }
                                // v1.16.2: `clear_errors` drops the pane back to the
                                // last good route (recovery, not a reload promise).
                                button {
                                    class: "btn btn-outline btn-sm mt-4",
                                    onclick: move |_| errors.clear_errors(),
                                    {t("err_dismiss")}
                                }
                            }
                        }
                    }
                },
                Router::<Route> {}
            }
        } else {
            // v1.27.21 (N15): the locale-neutral launch shell — no text, so no
            // locale can flash before the pref restore settles.
            div { class: "min-h-screen bg-background" }
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
    // v1.21.0 "Profiles" M3: the use-case wizard step. After a successful
    // MANUAL connect, if the home domain has no bound profile and the operator
    // hasn't skipped, show "What best describes your team?" — pick a preset,
    // see the knobs, apply. The auto-reconnect path stays silent (a returning
    // operator with a saved token is not the onboarding audience).
    // ponytail: binds the `global` home domain (the client is single-domain;
    // per-domain wizard targeting is `brain setup`'s job). Knob EDITING in the
    // wizard is a ceiling — the wizard picks, the API edits (POST /profiles).
    let wizard_profiles = use_signal(Vec::new as fn() -> Vec<crate::api::ProfileView>);
    let mut wizard_chosen = use_signal(|| 0usize);
    let api_signal = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let nav = navigator();

    // v1.16.6 M2: silent auto-reconnect on launch — if a token was previously
    // saved to the OS keyring, probe /health with it and jump straight to Review
    // instead of making the operator paste the token again. Web (no-op storage)
    // and first-run (no saved token) are unaffected. Best-effort: a stale/revoked
    // token just falls through to the normal connect form (no error flash).
    // `use_resource` (not `use_effect`) runs this exactly once on mount.
    {
        let mut api_signal = api_signal;
        let mut ui = ui;
        let mut busy = busy;
        use_resource(move || async move {
            if let Some(saved) = storage::load_token() {
                busy.set(true);
                let base = resolve_base("", page_origin().await);
                let client = ApiClient::with_principal(base, Some(saved), None);
                match client.health().await {
                    Ok(_) => {
                        api_signal.set(client);
                        ui.conn.set(Conn::Connected);
                        ui.writes_enabled.set(true);
                        ui.pending_reverify.set(false);
                        nav.replace(Route::Review {});
                    }
                    Err(_) => busy.set(false),
                }
            }
        });
    }

    // v1.17.0 M3.4: offline graceful — pre-fill the URL field with the last
    // successful base (a non-secret UI pref, same localStorage seam as the
    // theme/density prefs). The URL is not a credential, so it never touches
    // the OS keyring; storing it lets a returning operator reconnect to the
    // same self-hosted backend without retyping it after the auto-reconnect
    // (token path) falls through to this form. `use_effect` runs once on mount.
    use_effect(move || {
        spawn(async move {
            if let Some(last) = i18n::pref_load("last_base").await
                && let Some(prefill) = prefill_if_empty(&url(), &last)
            {
                url.set(prefill);
            }
        });
    });
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
        // v1.16.6 M2: snapshot the token to persist BEFORE `access_val` moves into
        // the ApiClient constructor. Persist only a real token (loopback never
        // clobbers a previously-saved remote one).
        let persist_token = access_val
            .clone()
            .filter(|t| storage::should_persist(Some(t)));
        let mut status = status;
        let mut busy = busy;
        let mut api_signal = api_signal;
        let mut ui = ui;
        let mut wizard_profiles = wizard_profiles;
        let mut wizard_chosen = wizard_chosen;
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
                Ok(h) => {
                    let cap = h
                        .capacity
                        .as_ref()
                        .map(|c| {
                            t_fmt(
                                "connected_capacity",
                                &[c.docs.to_string(), c.max_docs.to_string()],
                            )
                        })
                        .unwrap_or_default();
                    let msg = t_fmt("connected_v", &[h.version.clone(), cap, h.status.clone()]);
                    Ok(msg)
                }
                Err(e) => Err(t_fmt(
                    "could_not_reach",
                    &[base.clone(), crate::api::error_message(&e)],
                )),
            }));
            if matches!(status(), Some(Ok(_))) {
                // v1.16.6 M2: persist the token to the OS keyring on a successful
                // connect (only when a real token was provided — a loopback
                // connect never clobbers a previously-saved remote token). Best
                // effort: a keyring failure must not block the connect itself.
                if let Some(tok) = persist_token {
                    let _ = storage::save_token(&tok);
                }
                // v1.17.0 M3.4: remember the successful base (non-secret UI pref)
                // so an offline/returning connect pre-fills the URL field.
                i18n::pref_save("last_base", &base);
                // Write the client in so every panel re-fetches through it.
                // M1: a fresh connect is a clean connect — writes gate on the
                // probe's first green, but no recovery re-verify is pending.
                api_signal.set(client);
                ui.conn.set(Conn::Connected);
                ui.writes_enabled.set(true);
                ui.pending_reverify.set(false);
                // v1.21.0 "Profiles" M3: the wizard eligibility check — home
                // domain unbound + not previously skipped + presets reachable.
                // Any failure just falls through to Review (the wizard is an
                // enhancement, never a gate). pref_load is web-only; on desktop
                // the wizard reappears until a profile is bound (by design).
                let probe = api_signal();
                let eligible = matches!(
                    probe.domain_profile("global").await,
                    Ok(dp) if dp.profile.is_none()
                ) && i18n::pref_load("wizard_done").await.is_none();
                if eligible
                    && let Ok(ps) = probe.profiles().await
                    && !ps.is_empty()
                {
                    wizard_profiles.set(ps);
                    wizard_chosen.set(0);
                    busy.set(false);
                    return;
                }
                nav.replace(Route::Review {});
            }
            busy.set(false);
        });
    };

    // v1.21.0 "Profiles" M3: hoisted wizard reads + handlers (a `let` as a
    // direct child of an rsx `if` breaks the parser — see panels/data.rs).
    // The labels/knob lines are precomputed so the rsx body stays a pure
    // element list. `wizard_pick` is None until the eligibility check (post-
    // connect) fills the list — the wizard then renders below the connect card.
    let wizard_list = wizard_profiles();
    let wizard_pick = wizard_list
        .get(wizard_chosen().min(wizard_list.len().saturating_sub(1)))
        .cloned();
    let wizard_labels: Vec<String> = wizard_list
        .iter()
        .map(|p| format!("{} — {}", p.name, p.description.as_deref().unwrap_or("")))
        .collect();
    let wizard_kinds_label = wizard_pick
        .as_ref()
        .and_then(|p| p.kinds.clone())
        .map(|k| k.join(", "));
    let wizard_apply_name = wizard_pick.as_ref().map(|p| p.name.clone());
    let mut wizard_status = status;
    let on_wizard_apply = move |_| {
        let Some(name) = wizard_apply_name.clone() else {
            return;
        };
        let mut wizard_profiles = wizard_profiles;
        let api_signal = api_signal;
        let nav = nav;
        spawn(async move {
            let client = api_signal();
            match client.bind_profile("global", Some(&name)).await {
                Ok(_) => {
                    i18n::pref_save("wizard_done", "1");
                    wizard_profiles.set(Vec::new());
                    nav.replace(Route::Review {});
                }
                Err(e) => wizard_status.set(Some(Err(t_fmt(
                    "bind_failed",
                    &[crate::api::error_message(&e)],
                )))),
            }
        });
    };
    let on_wizard_skip = move |_| {
        let mut wizard_profiles = wizard_profiles;
        let nav = nav;
        spawn(async move {
            i18n::pref_save("wizard_done", "1");
            wizard_profiles.set(Vec::new());
            nav.replace(Route::Review {});
        });
    };
    let wizard_title_lbl = t("wizard_title");
    let wizard_hint_lbl = t("wizard_hint");
    let wizard_apply_lbl = t("wizard_apply");
    let wizard_skip_lbl = t("wizard_skip");

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
                            {t("connect_welcome")} }
                    }
                }
                div { class: "card",
                    div { class: "card-header", div { class: "card-title", {t("connect_title")} } }
                    div { class: "card-body space-y-4",
                        label { class: "block space-y-1",
                            span { class: "label", {t("backend_url")} }
                            input {
                                class: "input w-full",
                                value: "{url}",
                                oninput: move |e| url.set(e.value()),
                                placeholder: t("url_placeholder"),
                                "aria-label": t("aria_backend_url"),
                            }
                        }
                        if plaintext_http(&url()) {
                            p {
                                class: "text-warn text-sm",
                                "role": "alert",
                                {t("plaintext_http")}
                            }
                        }
                        label { class: "block space-y-1",
                            span { class: "label", {t("token_label")} }
                            input {
                                class: "input w-full",
                                "type": "password",
                                value: "{token}",
                                oninput: move |e| token.set(e.value()),
                                placeholder: if jwt_pair() { t("token_access_placeholder") } else { t("token_placeholder") },
                                "aria-label": t("aria_auth_token"),
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
                            {t("jwt_pair")}
                        }
                        if jwt_pair() {
                            label { class: "block space-y-1",
                                span { class: "label", {t("refresh_token_label")} }
                                input {
                                    class: "input w-full",
                                    "type": "password",
                                    value: "{refresh_token}",
                                    oninput: move |e| refresh_token.set(e.value()),
                                    placeholder: t("refresh_token_placeholder"),
                                    "aria-label": t("aria_refresh_token"),
                                }
                            }
                        }
                        button {
                            class: "btn btn-primary btn-md w-full",
                            disabled: busy(),
                            onclick: connect,
                            if busy() { {t("connecting")} } else { {t("connect_button")} }
                        }
                        if let Some(Ok(msg)) = &*status.read() {
                            p { class: "text-ok text-sm", "{msg}" }
                        }
                        if let Some(Err(msg)) = &*status.read() {
                            p { class: "text-danger text-sm", "{msg}" }
                        }
                    }
                }
                // v1.21.0 "Profiles" M3: the wizard step. A native <select> (real
                // keyboard/a11y for free) + the chosen preset's knobs + Apply/Skip.
                // The output is a live, configured store — no feature tours.
                if let Some(pick) = &wizard_pick {
                    div { class: "card",
                        div { class: "card-header",
                            div { class: "card-title", "{wizard_title_lbl}" }
                            p { class: "text-sm text-muted-foreground", "{wizard_hint_lbl}" }
                        }
                        div { class: "card-body space-y-3",
                            select {
                                class: "input w-full",
                                "aria-label": t("wizard_preset_aria"),
                                value: "{pick.name}",
                                onchange: move |e| {
                                    if let Some(i) = wizard_profiles()
                                        .iter()
                                        .position(|p| p.name == e.value())
                                    {
                                        wizard_chosen.set(i);
                                    }
                                },
                                for (p, label) in wizard_list.iter().zip(&wizard_labels) {
                                    option { value: "{p.name}", "{label}" }
                                }
                            }
                            dl { class: "grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm tabular",
                                if let Some(v) = &pick.default_access_scope {
                                    dt { class: "text-muted-foreground", {t("wizard_knob_scope")} } dd { "{v}" }
                                }
                                if let Some(v) = &pick.pii_mode {
                                    dt { class: "text-muted-foreground", {t("wizard_knob_pii")} } dd { "{v}" }
                                }
                                if let Some(v) = pick.retention_label() {
                                    dt { class: "text-muted-foreground", {t("wizard_knob_retention")} } dd { "{v}" }
                                }
                                if let Some(v) = &pick.audit_level {
                                    dt { class: "text-muted-foreground", {t("wizard_knob_audit")} } dd { "{v}" }
                                }
                                if let Some(v) = &wizard_kinds_label {
                                    dt { class: "text-muted-foreground", {t("wizard_knob_kinds")} } dd { "{v}" }
                                }
                                if let Some(v) = pick.legal_hold_default {
                                    dt { class: "text-muted-foreground", {t("wizard_knob_hold")} } dd { "{v}" }
                                }
                            }
                            p { class: "text-xs text-ink-faint",
                                {t("wizard_defaults_note")}
                            }
                            div { class: "flex gap-2",
                                button {
                                    class: "btn btn-primary btn-md flex-1",
                                    onclick: on_wizard_apply,
                                    "{wizard_apply_lbl}"
                                }
                                button {
                                    class: "btn btn-md",
                                    onclick: on_wizard_skip,
                                    "{wizard_skip_lbl}"
                                }
                            }
                        }
                    }
                }
                p { class: "text-center text-xs text-ink-faint",
                    {t("install_hint")}
                }
                // v1.16.8 M6.2 — data-flow transparency on the connect screen.
                // States exactly what the client sends / stores / never does.
                details { class: "card card-body text-xs text-muted-foreground",
                    summary { class: "cursor-pointer text-sm text-muted-foreground", {t("privacy_title")} }
                    p { class: "mt-2 text-muted-foreground", {t("privacy_sends")} }
                    ul { class: "list-disc pl-4 mt-1",
                        li { {t("privacy_sends_1")} }
                        li { {t("privacy_sends_2")} }
                    }
                    p { class: "mt-2 text-muted-foreground", {t("privacy_stores")} }
                    ul { class: "list-disc pl-4 mt-1",
                        li { {t("privacy_stores_1")} }
                        li { {t("privacy_stores_2")} }
                    }
                    p { class: "mt-2 text-muted-foreground", {t("privacy_not")} }
                    ul { class: "list-disc pl-4 mt-1",
                        li { {t("privacy_not_1")} }
                        li { {t("privacy_not_2")} }
                        li { {t("privacy_not_3")} }
                        li { {t("privacy_not_4")} }
                    }
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
    let mut api = use_context::<Signal<ApiClient>>();
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
    let palette_open = (ui.palette_open)();

    // v1.17.6 M2.6: connect-first guard. The shell pages need a configured
    // client; if it's unconfigured (first-run or after sign-out) on a shell
    // route, redirect to Connect. Connect lives OUTSIDE this layout, so this
    // effect only runs for the authenticated pages — no loop. Reading `api()`
    // subscribes AppShell to the client signal, so a connect/sign-out re-runs it.
    let client_configured = api().is_configured();
    use_effect(move || {
        if !client_configured {
            navigator().replace(Route::Connect {});
        }
    });

    // v1.16.8 M1: localize the shell chrome. Precomputed so the rsx can use
    // `"{var}"` / bare-expression interpolation — a literal `t("key")` call
    // inside a formatted string (nested quotes) breaks the rsx parser. Reading
    // `t()` here subscribes this component to locale changes, so switching the
    // locale re-renders the shell in the new language.
    let conn_text = t(conn_label(conn));
    // M5: locale-aware digit grouping on the shell counts.
    let pending_label = format!("{} {pending}", i18n::format_number(pending as u64));
    // v1.20.0 M3: the offline-queue badge (actions awaiting replay).
    let queued = queue::queue().read().len();
    let queued_label = (queued > 0).then(|| format!("{} {queued}", t("nav_queued")));
    let queued_title = t("nav_queued_title");
    let flags_label = format!(
        "{} {}",
        i18n::format_number(security_badge as u64),
        t("flags")
    );
    let principal_label = principal
        .as_ref()
        .map(|p| format!("{} {p}", t("acting_as")));
    let theme_label = t("theme_label");
    let density_label = t("density_label");
    let locale_label = t("locale_label");
    let nav_overview = t("nav_overview");
    let nav_review = t("nav_review");
    let nav_recall = t("nav_recall");
    let nav_graph = t("nav_graph");
    let nav_create = t("nav_create");
    let nav_subjects = t("nav_subjects");
    let nav_security = t("nav_security");
    let nav_audit = t("nav_audit");
    let nav_health = t("nav_health");
    let nav_data = t("nav_data");
    let nav_ump = t("nav_ump");
    let nav_system = t("nav_system");
    let nav_ops = t("nav_ops");
    let nav_register = t("nav_register");
    let nav_clients = t("nav_clients");
    let review_title = t("review_title");
    let sign_out = t("sign_out");
    let loopback = t("loopback");
    let audit_chain = t("audit_chain");
    let disconnected = t("disconnected");
    let reverifying = t("reverifying");

    // v1.23.0 M3 "Roles": the resolved role names gate which nav panels render
    // (defense-in-depth — the server still enforces the underlying endpoints).
    let nav_roles = api().roles();
    // v1.27.11 "Console": which BPO dashboard view this principal gets. The
    // Clients route is only reachable from the nav when a console posture is
    // resolved (`client-auditor` → their own dashboard; `bpo-ops`/admin → the
    // all-clients board). The panel also checks it internally.
    let console_view = crate::role::console_view(&nav_roles);

    // v1.28.1 M4 (F-34): the parked destructive actions — shown as a review
    // banner while the connection is up (the reconnect window in which they
    // could auto-fire). The banner's decisions write the queue directly.
    let parked = {
        let q = queue::queue();
        let qr = q.read();
        crate::queue::parked(&qr)
    };
    let parked_shown = conn == Conn::Connected && !parked.is_empty();

    // v1.16.7 M5: cmd/ctrl+K toggles the command palette. Handled on the shell
    // root (focused by default when the app has focus); the palette's own input
    // captures its keys while open.
    //
    // Session-first shell — ⌘B collapses the nav rail to an icon
    // strip (persisted pref); Esc closes it.
    let mut sidebar_collapsed = use_signal(|| false);
    use_future(move || async move {
        if i18n::pref_load("sidebar_collapsed").await.as_deref() == Some("1") {
            sidebar_collapsed.set(true);
        }
    });
    let shell_keys = move |e: Event<KeyboardData>| {
        let mut ui = ui;
        let mut sidebar_collapsed = sidebar_collapsed;
        let mod_pressed =
            e.modifiers().contains(Modifiers::CONTROL) || e.modifiers().contains(Modifiers::SUPER);
        if mod_pressed && let Key::Character(c) = e.key() {
            if c.eq_ignore_ascii_case("k") {
                ui.palette_open.set(!(ui.palette_open)());
            } else if c.eq_ignore_ascii_case("b") {
                let next = !sidebar_collapsed();
                sidebar_collapsed.set(next);
                i18n::pref_save("sidebar_collapsed", if next { "1" } else { "0" });
            }
        } else if e.key() == Key::Escape && !(ui.palette_open)() {
            sidebar_collapsed.set(false);
        }
    };

    rsx! {
        div { class: "flex min-h-screen bg-background text-foreground", onkeydown: shell_keys,
            // Fixed sidebar: brand + primary nav + identity footer.
            // collapsible (⌘B / Esc); collapsed = icon strip with the pending
            // count + running dot kept visible.
            aside {
                class: if sidebar_collapsed() {
                    "nav-rail nav-collapsed sticky top-0 flex h-screen w-14 shrink-0 flex-col border-r border-border bg-card"
                } else {
                    "nav-rail sticky top-0 flex h-screen w-56 shrink-0 flex-col border-r border-border bg-card"
                },
                "aria-label": t("aria_primary_nav"),
                "aria-expanded": "{!sidebar_collapsed()}",
                div { class: "flex items-center gap-2 border-b border-border px-4 h-14",
                    div { class: "flex size-8 items-center justify-center rounded-md bg-accent/15 text-accent",
                        span { class: "font-mono text-sm font-bold", "b" }
                    }
                    if !sidebar_collapsed() {
                        span { class: "nav-label font-semibold tracking-tight", "brain" }
                    }
                }
                nav { class: "flex-1 overflow-y-auto p-3 nav",
                    // M2.1: F-pattern anchor — pending review count on the rail.
                    div { class: "mb-1 flex items-center justify-between px-2.5 text-xs text-muted-foreground",
                        span { "{review_title}" }
                        span { class: "tabular", "{pending_label}" }
                    }
                    NavLink { to: Route::Overview {}, "{nav_overview}" }
                    NavLink { to: Route::Review {}, "{nav_review}" }
                    NavLink { to: Route::Recall {}, "{nav_recall}" }
                    NavLink { to: Route::Graph {}, "{nav_graph}" }
                    if role::role_can_see(&nav_roles, "subjects") {
                        NavLink { to: Route::Subjects {}, "{nav_subjects}" }
                    }
                    if role::role_can_see(&nav_roles, "security") {
                        NavLink { to: Route::Security {}, badge: Some(security_badge), "{nav_security}" }
                    }
                    if role::role_can_see(&nav_roles, "audit") {
                        NavLink { to: Route::Audit { since: None, principal: None }, dirty: audit_dirty, "{nav_audit}" }
                    }
                    NavLink { to: Route::Create {}, "{nav_create}" }
                    NavLink { to: Route::Health {}, "{nav_health}" }
                    if role::role_can_see(&nav_roles, "data") {
                        NavLink { to: Route::Data {}, "{nav_data}" }
                    }
                    NavLink { to: Route::Ump {}, "{nav_ump}" }
                    NavLink { to: Route::System {}, "{nav_system}" }
                    NavLink { to: Route::Ops {}, "{nav_ops}" }
                    NavLink { to: Route::Register {}, "{nav_register}" }
                    if console_view != crate::role::ConsoleView::Undefined {
                        NavLink { to: Route::Clients {}, "{nav_clients}" }
                    }
                }
                div { class: "border-t border-border p-3",
                div { class: "flex items-center gap-2 text-sm",
                    span { class: conn_dot(conn), "●" }
                    span { class: "text-xs text-muted-foreground", "{conn_text}" }
                }
                if let Some(p) = &principal_label {
                    p { class: "mt-1 truncate text-xs text-muted-foreground tabular", "{p}" }
                } else {
                    p { class: "mt-1 text-xs text-ink-faint", "{loopback}" }
                }
                // v1.16.7 M7.1: explicit logout. Clears the OS-keyring token,
                // resets the in-memory ApiClient, and returns to Connect so a
                // signed-out operator never auto-reconnects with a saved token.
                if api().is_configured() {
                    button {
                        class: "btn btn-ghost btn-sm w-full mt-2 justify-center",
                        onclick: move |_| {
                            storage::delete_token();
                            api.set(ApiClient::unconfigured());
                            navigator().replace(Route::Connect {});
                        },
                        "{sign_out}"
                    }
                }
                }
            }
            // v1.16.6 M3: mobile bottom tab bar. `position: fixed`; CSS shows it
            // <640px and hides the rail (pure breakpoint swap, same Routable).
            // Both surfaces use the same NavLink targets so a11y nav is identical.
            nav {
                class: "tab-bar",
                "aria-label": t("aria_primary_nav_mobile"),
                TabLink { to: Route::Overview {}, "{nav_overview}" }
                TabLink { to: Route::Review {}, "{nav_review}" }
                TabLink { to: Route::Recall {}, "{nav_recall}" }
                TabLink { to: Route::Graph {}, "{nav_graph}" }
                if role::role_can_see(&nav_roles, "subjects") {
                    TabLink { to: Route::Subjects {}, "{nav_subjects}" }
                }
                if role::role_can_see(&nav_roles, "security") {
                    TabLink { to: Route::Security {}, badge: Some(security_badge), "{nav_security}" }
                }
                if role::role_can_see(&nav_roles, "audit") {
                    TabLink { to: Route::Audit { since: None, principal: None }, dirty: audit_dirty, "{nav_audit}" }
                }
                TabLink { to: Route::Create {}, "{nav_create}" }
                TabLink { to: Route::Health {}, "{nav_health}" }
                if role::role_can_see(&nav_roles, "data") {
                    TabLink { to: Route::Data {}, "{nav_data}" }
                }
                TabLink { to: Route::Ump {}, "{nav_ump}" }
                TabLink { to: Route::System {}, "{nav_system}" }
                TabLink { to: Route::Ops {}, "{nav_ops}" }
                TabLink { to: Route::Register {}, "{nav_register}" }
                if console_view != crate::role::ConsoleView::Undefined {
                    TabLink { to: Route::Clients {}, "{nav_clients}" }
                }
            }
            div { class: "flex min-w-0 flex-1 flex-col",
                // Slim top bar: connection + pending summary + prefs.
                header { class: "sticky top-0 z-10 flex h-14 items-center gap-3 border-b border-border bg-background/80 px-4 backdrop-blur",
                    span {
                        class: "font-mono text-sm",
                        "aria-label": t("aria_connection_status"),
                        span { class: conn_dot(conn), "●" }
                        " {conn_text}"
                    }
                    span { class: "text-sm text-muted-foreground tabular",
                        "{pending_label}"
                    }
                    if security_badge > 0 {
                        span { class: "badge badge-warn", "{flags_label}" }
                    }
                    if audit_dirty {
                        span { class: "badge badge-danger", "{audit_chain}" }
                    }
                    // v1.20.0 M3: actions queued for replay while offline.
                    if let Some(l) = queued_label {
                        span {
                            class: "badge badge-warn font-mono tabular",
                            title: "{queued_title}",
                            role: "status", "aria-live": "polite",
                            "{l}"
                        }
                    }
                    // v1.16.8 M3/M4/M1: theme + density toggles + locale switch.
                    // Non-sensitive UI prefs, persisted to localStorage; changing
                    // them re-runs the root effects (tokens/dir swap, no reload).
                    div { class: "ml-auto flex items-center gap-1.5",
                        button {
                            class: "btn btn-ghost btn-sm",
                            "aria-label": "{theme_label}",
                            title: "{theme_label}",
                            onclick: move |_| {
                                // v1.20.0 M1: dark → light → system → dark
                                // cycle; "system" follows the OS via CSS
                                // `prefers-color-scheme` (no JS).
                                let current = i18n::theme()();
                                let next = i18n::THEME_MODES
                                    .iter()
                                    .position(|m| *m == current)
                                    .map(|i| i18n::THEME_MODES[(i + 1) % i18n::THEME_MODES.len()])
                                    .unwrap_or("system");
                                i18n::theme().set(next);
                                i18n::pref_save("theme", next);
                            },
                            if i18n::theme()() == "dark" { "☾" }
                            else if i18n::theme()() == "light" { "☀" }
                            else { "◐" }
                        }
                        button {
                            class: "btn btn-ghost btn-sm",
                            "aria-label": "{density_label}",
                            title: "{density_label}",
                            onclick: move |_| {
                                let next = if i18n::density()() == "compact" { "comfortable" } else { "compact" };
                                i18n::density().set(next);
                                i18n::pref_save("density", next);
                            },
                            if i18n::density()() == "compact" { "comfortable" } else { "compact" }
                        }
                        select {
                            class: "select h-7 w-auto px-1.5 text-xs",
                            "aria-label": "{locale_label}",
                            value: "{i18n::locale()()}",
                            onchange: move |e| {
                                let next = i18n::pick_locale(&e.value());
                                i18n::locale().set(next);
                                i18n::pref_save("locale", next);
                            },
                            for l in i18n::SUPPORTED_LOCALES {
                                option { value: l, "{l}" }
                            }
                        }
                    }
                    if let Some(p) = &principal {
                        span { class: "text-xs text-muted-foreground tabular", "{p}" }
                    }
                }
                if parked_shown {
                    div { class: "px-4 pt-3",
                        crate::replay::ReplayReview {
                            items: parked.clone(),
                            on_update: crate::queue::replace_parked,
                        }
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
                "{disconnected}"
            }
        }
        if !writes && conn == Conn::Connected && pending_reverify {
            div {
                class: "border-b border-info/40 bg-info/10 px-4 py-1 text-sm text-info",
                role: "status",
                "{reverifying}"
            }
        }
        // v1.16.7 M5: the command palette overlay (cmd/ctrl+K). Rendered last so
        // it sits above the drawer.
        if palette_open {
            CommandPalette { }
        }
    }
}

/// v1.17.6 M1: command palette v2. A fused nav + lookup + action surface
/// (the otf-kit / Commander contract) so every panel and action is one
/// keystroke away BEFORE its panel exists. `Command` is a flat tagged enum;
/// the pure cores (`palette_group` / `command_keywords` / `palette_lookup` /
/// `remember_recent` / `destructive_action`) are testable without a Dioxus
/// runtime. New panels (v1.17.7/v1.17.8) register into `palette_commands` +
/// `command_label`/`command_keywords` — this enum IS the single source of truth
/// (M1.5).
/// `#[allow(dead_code)]` on the reserved `Lookup`/`Run` variants below (nothing
/// constructs them until v1.17.7/v1.17.8) — the label/keyword/group/destructive
/// arms are already live, so the surface is reserved, not unfinished.
#[allow(dead_code)]
#[derive(Clone, PartialEq)]
enum Command {
    /// "Go to" — every route with a non-detail page (rail + new panels).
    Navigate(Route),
    /// "Lookup" — client-held ids, resolvable instantly.
    Lookup(Lookup),
    /// "Run" — actions (export / reindex / refresh / trace).
    Run(RunAction),
    /// Stays a Run-group member.
    SignOut,
}

/// v1.17.6 M1.1: lookup rows are client-held ids. The type + group ship now;
/// live ids arrive when a panel feeds them in (the honest ceiling in the plan).
/// `#[allow(dead_code)]`: nothing constructs Lookup/Run rows until v1.17.7/
/// v1.17.8 panels feed them — the match arms (label/keywords/group/destructive)
/// are already live, so the surface is reserved, not unfinished.
#[allow(dead_code)]
#[derive(Clone, PartialEq)]
enum Lookup {
    Proposal(i64),
    Chunk(i64),
    Entity(String),
}

/// v1.17.6 M1.1: Run-group actions. Reindex is destructive (two-step confirm).
/// `#[allow(dead_code)]`: same reservation as `Lookup` — arms are wired, the
/// constructors come with the v1.17.7/v1.17.8 action rows.
#[allow(dead_code)]
#[derive(Clone, PartialEq)]
pub(crate) enum RunAction {
    ExportAudit,
    ExportUmp,
    Reindex,
    Refresh(String),
    OpenTrace(i64),
}

/// v1.17.6 M1.2: the group label as an i18n key (render via `t()`), so the
/// pure core stays locale-agnostic. SignOut is a Run-group member.
fn palette_group(c: &Command) -> &'static str {
    match c {
        Command::Navigate(_) => "palette_go_to",
        Command::Lookup(_) => "palette_lookup",
        Command::Run(_) | Command::SignOut => "palette_run",
    }
}

/// v1.17.6 M1.2: the fuzzy-search index — label words + aliases. Substring
/// match over these drives `palette_lookup`.
fn command_keywords(c: &Command) -> &'static [&'static str] {
    match c {
        Command::Navigate(Route::Overview {}) => &["overview", "home", "dashboard"],
        Command::Navigate(Route::Review {}) => &["review", "queue", "pending", "approve"],
        Command::Navigate(Route::Recall {}) => &["recall", "search", "query"],
        Command::Navigate(Route::Graph {}) => &["graph", "traverse", "entity", "kg"],
        Command::Navigate(Route::Create {}) => &["create", "ingest", "write", "procedure"],
        Command::Navigate(Route::Subjects {}) => &[
            "subjects",
            "dsar",
            "erasure",
            "delete",
            "privacy",
            "certificate",
        ],
        Command::Navigate(Route::Security {}) => &["security", "quarantine", "flags", "auth"],
        Command::Navigate(Route::Audit {
            since: None,
            principal: None,
        }) => &["audit", "log", "history", "chain"],
        Command::Navigate(Route::Health {}) => &["health", "status", "capacity", "service"],
        Command::Navigate(Route::Data {}) => &["data", "purge", "export", "retention", "rights"],
        Command::Navigate(Route::Ump {}) => &["ump", "portability", "protocol", "interop"],
        Command::Navigate(Route::System {}) => &["system", "admin", "console", "domains"],
        Command::Navigate(Route::Ops {}) => &[
            "ops",
            "operations",
            "queue",
            "sla",
            "clock",
            "deadline",
            "flagged",
        ],
        Command::Navigate(Route::Register {}) => &[
            "register",
            "ledger",
            "provenance",
            "origin",
            "who",
            "ownership",
        ],
        Command::Navigate(Route::Clients {}) => &[
            "clients",
            "console",
            "bpo",
            "dashboard",
            "auditor",
            "register",
        ],
        Command::SignOut => &["signout", "sign out", "logout", "exit"],
        Command::Lookup(Lookup::Proposal(_)) => &["proposal", "propose", "approve"],
        Command::Lookup(Lookup::Chunk(_)) => &["chunk", "memory", "get"],
        Command::Lookup(Lookup::Entity(_)) => &["entity", "graph", "traverse"],
        Command::Run(RunAction::ExportAudit) => &["export", "audit", "download"],
        Command::Run(RunAction::ExportUmp) => &["export", "ump", "portability"],
        Command::Run(RunAction::Reindex) => &["reindex", "rebuild", "refresh"],
        Command::Run(RunAction::Refresh(_)) => &["refresh", "reload"],
        Command::Run(RunAction::OpenTrace(_)) => &["trace", "recall", "explain"],
        Command::Navigate(_) => &["open"],
    }
}

/// v1.17.6 M1.5 pure: the static command registration — the single source of
/// truth the palette + the M1.5 guard test both read. Every non-detail route +
/// (only when a client is configured) Sign out. Lookup/Run rows are added by
/// the panels that own them (v1.17.7/v1.17.8).
fn palette_commands(configured: bool) -> Vec<Command> {
    let mut v = vec![
        Command::Navigate(Route::Overview {}),
        Command::Navigate(Route::Review {}),
        Command::Navigate(Route::Recall {}),
        Command::Navigate(Route::Graph {}),
        Command::Navigate(Route::Subjects {}),
        Command::Navigate(Route::Security {}),
        Command::Navigate(Route::Audit {
            since: None,
            principal: None,
        }),
        Command::Navigate(Route::Create {}),
        Command::Navigate(Route::Health {}),
        Command::Navigate(Route::Data {}),
        Command::Navigate(Route::Ump {}),
        Command::Navigate(Route::System {}),
        Command::Navigate(Route::Ops {}),
        Command::Navigate(Route::Register {}),
        Command::Navigate(Route::Clients {}),
    ];
    if configured {
        v.push(Command::SignOut);
    }
    v
}

/// v1.17.6 M1.2 pure: grouped keyword search with the Recent group prepended.
/// Regular groups cap at 5 per group (the Linear/Raycast convention); the
/// Recent group shows the persisted recents (resolved back to live commands so
/// a re-run always acts on the current set). Empty needle returns every group
/// (capped); a typed needle filters, and the Recent group hides while filtering
/// (you're searching the full set, not recents).
fn palette_lookup(
    commands: &[Command],
    needle: &str,
    recents: &[String],
) -> Vec<(&'static str, Vec<Command>)> {
    let mut out: Vec<(&'static str, Vec<Command>)> = Vec::new();
    let n = needle.trim().to_lowercase();
    if n.is_empty() {
        let recent_cmds: Vec<Command> = recents
            .iter()
            .filter_map(|label| {
                commands
                    .iter()
                    .find(|c| command_label(c) == label.as_str())
                    .cloned()
            })
            .collect();
        if !recent_cmds.is_empty() {
            out.push(("palette_recent", recent_cmds));
        }
    }
    let mut groups: Vec<(&'static str, Vec<Command>)> = Vec::new();
    for c in commands {
        let hit = n.is_empty()
            || command_keywords(c)
                .iter()
                .any(|k| k.to_lowercase().contains(&n))
            || command_label(c).to_lowercase().contains(&n);
        if !hit {
            continue;
        }
        let group = palette_group(c);
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, list)) if list.len() < 5 => list.push(c.clone()),
            Some(_) => {} // per-group cap reached
            None => groups.push((group, vec![c.clone()])),
        }
    }
    out.extend(groups);
    out
}

/// v1.17.6 M1.3 pure: recents append + cap. Re-running a label moves it to the
/// front (dedup by label); capped at 8. The caller persists the comma-joined
/// Vec via `i18n::pref_save("palette_recent", …)`.
fn remember_recent(recent: &[String], id: &str) -> Vec<String> {
    let mut out: Vec<String> = recent.to_vec();
    out.retain(|s| s != id);
    out.insert(0, id.to_string());
    out.truncate(8);
    out
}

/// v1.17.6 M1.2 pure: is this Run action destructive (danger token + a second
/// Enter to confirm)?
pub(crate) fn destructive_action(a: &RunAction) -> bool {
    matches!(a, RunAction::Reindex)
}

/// M5 pure: a command's search/display label.
fn command_label(c: &Command) -> &'static str {
    match c {
        Command::Navigate(Route::Overview {}) => "overview_title",
        Command::Navigate(Route::Review {}) => "review_title",
        Command::Navigate(Route::Recall {}) => "recall_title",
        Command::Navigate(Route::Graph {}) => "graph_title",
        Command::Navigate(Route::Create {}) => "create_title",
        Command::Navigate(Route::Subjects {}) => "subjects_title",
        Command::Navigate(Route::Security {}) => "security_title",
        Command::Navigate(Route::Audit {
            since: None,
            principal: None,
        }) => "audit_title",
        Command::Navigate(Route::Health {}) => "health_title",
        Command::Navigate(Route::Data {}) => "data_title",
        Command::Navigate(Route::Ump {}) => "ump_title",
        Command::Navigate(Route::System {}) => "system_title",
        Command::Navigate(Route::Ops {}) => "ops_title",
        Command::Navigate(Route::Register {}) => "register_title",
        Command::Navigate(Route::Clients {}) => "clients_title",
        Command::SignOut => "sign_out",
        Command::Lookup(Lookup::Proposal(_)) => "palette_open_proposal",
        Command::Lookup(Lookup::Chunk(_)) => "palette_open_chunk",
        Command::Lookup(Lookup::Entity(_)) => "palette_open_entity",
        Command::Run(RunAction::ExportAudit) => "palette_export_audit",
        Command::Run(RunAction::ExportUmp) => "palette_export_ump",
        Command::Run(RunAction::Reindex) => "reindex_title",
        Command::Run(RunAction::Refresh(_)) => "refresh_label",
        Command::Run(RunAction::OpenTrace(_)) => "palette_open_trace",
        // The deep-link / Connect variants never appear in the palette list.
        Command::Navigate(_) => "palette_open",
    }
}

/// v1.17.6 M1.4: the command palette v2. cmd/ctrl+K overlay; grouped sections
/// (Recent / Go to / Lookup / Run), keyword filter, keyboard-navigable
/// (↑/↓/Enter/Esc/Tab, `/` re-focus), and a two-step confirm for destructive
/// actions. Recents persist through the existing `i18n::pref_save` seam.
#[component]
fn CommandPalette() -> Element {
    let ui = use_context::<UiState>();
    let mut api = use_context::<Signal<ApiClient>>();
    let configured = api().is_configured();
    let nav = navigator();
    let mut needle = use_signal(String::new);
    let mut cursor = use_signal(|| 0usize);
    // v1.17.6 M1.3: recents load once per open (the palette mounts fresh each
    // time `palette_open` flips true). Stored comma-joined, never a secret.
    let mut recents = use_signal(Vec::<String>::new);
    use_future(move || async move {
        if let Some(raw) = i18n::pref_load("palette_recent").await {
            recents.set(raw.split(',').map(str::to_string).collect());
        }
    });
    // v1.17.6 M1.4: the destructive-confirm step (Some = awaiting 2nd Enter).
    let confirm = use_signal(|| None::<Command>);

    let confirm_destructive = t("confirm_destructive");
    let palette_placeholder = t("palette_placeholder");
    let palette_filter_aria = t("palette_filter_aria");
    let palette_no_match = t("palette_no_match");
    let confirm_text = confirm()
        .as_ref()
        .map(|c| format!("{confirm_destructive}: {}", t(command_label(c))));
    let commands = palette_commands(configured);
    let groups = if confirm().is_some() {
        Vec::new() // confirm state replaces the list
    } else {
        palette_lookup(&commands, &needle(), &recents())
    };
    // Flatten for cursor indexing; group headers render as labels, not items.
    let flat: Vec<Command> = groups
        .iter()
        .flat_map(|(_, cmds)| cmds.iter().cloned())
        .collect();
    if cursor() >= flat.len().max(1) {
        cursor.set(0);
    }
    // M1.2: flatten the groups into owned (index, optional-header, command)
    // rows so the `for` body needs no `let` and the onclick/onmouseenter
    // closures capture only Copy (index) / owned (command) values. The header
    // is `Some(group)` for a group's first row only.
    let rows: Vec<(usize, Option<&'static str>, Command)> = {
        let mut rows = Vec::new();
        let mut idx = 0usize;
        for (group, cmds) in groups {
            for (j, c) in cmds.into_iter().enumerate() {
                rows.push((idx, (j == 0).then_some(group), c));
                idx += 1;
            }
        }
        rows
    };

    // Run a command for real (no destructive gate — the confirm path calls it).
    // Signal writes happen on Copy handles / inside `spawn`, so `run` stays `Fn`
    // + `Copy`, which every event handler grabs without a borrow conflict.
    let run = move |c: Command| {
        match &c {
            Command::Navigate(route) => {
                let _ = nav.push(route.clone());
            }
            Command::SignOut => {
                spawn(async move {
                    storage::delete_token();
                    *api.write() = ApiClient::unconfigured();
                });
            }
            // Lookup / non-destructive Run rows arrive with their owning panels
            // (v1.17.7/v1.17.8); nothing to wire yet, just close.
            _ => {}
        }
        let recent = remember_recent(&recents(), command_label(&c));
        let joined = recent.join(",");
        let mut recents = recents;
        let mut ui = ui;
        spawn(async move {
            i18n::pref_save("palette_recent", &joined);
            recents.set(recent);
            *ui.palette_open.write() = false;
        });
    };
    // The selection gate: a destructive Run → confirm step, else run.
    let select = move |c: &Command| {
        let mut confirm = confirm;
        let mut cursor = cursor;
        if let Command::Run(ra) = c
            && destructive_action(ra)
        {
            confirm.set(Some(c.clone()));
            cursor.set(0);
            return;
        }
        run(c.clone());
    };

    let cursor_ui = ui;
    let confirm_ui = confirm;
    let run_ui = run;
    let select_ui = select;
    let flat_ui = flat.clone();

    rsx! {
        crate::Modal {
            label: t("palette_modal_label").to_string(),
            trap: ".command-palette".to_string(),
            initial_focus: ".command-palette input".to_string(),
            close_on_overlay: true,
            on_close: move |_| {
                let mut cursor_ui = cursor_ui;
                let mut confirm_ui = confirm_ui;
                if confirm_ui().is_some() {
                    confirm_ui.set(None);
                } else {
                    *cursor_ui.palette_open.write() = false;
                }
            },
            div {
                class: "command-palette card w-full max-w-md bg-popover shadow-xl mt-24",
                onmousedown: move |e| e.stop_propagation(),
                onkeydown: move |e| {
                    let mut cursor = cursor;
                    let mut confirm_ui = confirm_ui;
                    let select_ui = select_ui;
                    let run_ui = run_ui;
                    let flat = flat_ui.clone();
                    match e.key() {
                        // Esc + Tab are owned by the Modal shell (two-step Esc
                        // lives in `on_close`; the focus trap is v1.16.7's).
                        Key::ArrowDown => {
                            cursor.set((cursor() + 1).min(flat.len().saturating_sub(1)))
                        }
                        Key::ArrowUp => cursor.set(cursor().saturating_sub(1)),
                        Key::Enter => {
                            if let Some(c) = confirm_ui().clone() {
                                confirm_ui.set(None);
                                run_ui(c);
                            } else {
                                let idx = cursor();
                                if let Some(c) = flat.get(idx) {
                                    select_ui(c);
                                }
                            }
                        }
                        Key::Character(k) if k == "/" => {
                            // `/` re-focuses the search input from anywhere in the
                            // overlay (no preventDefault — a `/` typed into the
                            // input still registers as search text).
                            spawn(async move {
                                let _ = document::eval(
                                    "const i=document.querySelector('.command-palette input'); if(i) i.focus();",
                                ).await;
                            });
                        }
                        _ => {}
                    }
                },
                input {
                    class: "input w-full border-b border-border rounded-b-none",
                    placeholder: "{palette_placeholder}",
                    value: "{needle}",
                    oninput: move |e| {
                        let mut cursor = cursor;
                        let mut confirm = confirm;
                        needle.set(e.value());
                        cursor.set(0);
                        confirm.set(None);
                    },
                    onmounted: move |el| {
                        let el = el.data();
                        spawn(async move { let _ = el.set_focus(true).await; });
                    },
                    "aria-label": "{palette_filter_aria}",
                }
                ul { class: "max-h-80 overflow-y-auto p-1.5",
                    // v1.17.6 M1.4: destructive confirm replaces the list.
                    if let Some(text) = confirm_text {
                        li {
                            class: "px-3 py-2 text-sm text-danger",
                            "role": "status",
                            "aria-live": "polite",
                            "{text}"
                        }
                    } else if flat.is_empty() {
                        li { class: "px-3 py-2 text-sm text-muted-foreground", "{palette_no_match}" }
                    } else {
                        for (global, header, c) in rows {
                            if let Some(g) = header {
                                li { class: "px-3 pt-2 pb-1 text-xs uppercase tracking-wide text-ink-faint", role: "presentation", "{t(g)}" }
                            }
                            li {
                                class: if cursor() == global {
                                    "cursor-pointer rounded-md px-3 py-2 text-sm bg-accent/10 text-accent"
                                } else {
                                    "cursor-pointer rounded-md px-3 py-2 text-sm"
                                },
                                onmouseenter: move |_| {
                                    let mut cursor = cursor;
                                    cursor.set(global);
                                },
                                onclick: {
                                    let c = c.clone();
                                    move |_ev| {
                                        let select_ui = select_ui;
                                        select_ui(&c);
                                    }
                                },
                                "aria-label": t(command_label(&c)),
                                {t(command_label(&c))}
                            }
                        }
                    }
                }
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

/// v1.16.6 M3: the mobile bottom-tab variant of `NavLink`. Same `Routable`
/// targets; larger touch surface (min-height 44px via `.tab-link`) + the badge
/// floats above the label (there's no rail "row" to badge into on a phone).
#[component]
fn TabLink(
    to: Route,
    badge: Option<u64>,
    #[props(default)] dirty: bool,
    children: Element,
) -> Element {
    rsx! {
        Link {
            to,
            class: "tab-link relative",
            active_class: "tab-link-active",
            {children}
            if let Some(n) = badge {
                if n > 0 {
                    span { class: "tab-badge", "{n}" }
                }
            }
            if dirty {
                span { class: "tab-badge tab-badge-alert", "!" }
            }
        }
    }
}

/// v1.16.7 M7.3: a real Tab-cycling focus trap for a dialog, hand-rolled
/// (`dx components add dialog` needs the registry, which is unreachable here —
/// ponytail ceiling). Runs a small JS snippet via `document::eval` to gather
/// the focusable descendants of the dialog and wrap Tab/Shift+Tab between the
/// first and last, so keyboard focus can't escape into the page behind the
/// modal. The host dialog keeps its own `onkeydown` for Escape (not handled
/// here, so callers can own it).
async fn focus_trap(dialog_selector: &str, is_shift_tab: bool) {
    let js = format!(
        r#"
const root = document.querySelector({dialog_selector:?});
if (root) {{
  const f = Array.from(root.querySelectorAll(
    'a[href], button:not([disabled]), textarea, input, select, [tabindex]:not([tabindex="-1"])'));
  if (f.length === 0) return;
  const i = f.indexOf(document.activeElement);
  const next = {is_shift_tab} ? (i <= 0 ? f[f.length - 1] : f[i - 1]) : (i < 0 ? f[0] : f[(i + 1) % f.length]);
  next.focus();
  if (next.tagName === 'A') next.setAttribute('tabindex', '0');
}}
"#
    );
    let _ = document::eval(&js).await;
}

/// v1.27.20 "Console" (M2.1): the shared modal shell — one component for every
/// dialog so keyboard parity is the DEFAULT (the pre-fix editors had Esc-only;
/// focus could Tab out into the page behind them). Renders the overlay +
/// dialog semantics + Esc-to-close + the v1.16.7 Tab-cycling `focus_trap` +
/// autofocus of `initial_focus` on open. `close_on_overlay` reproduces the
/// palette's click-outside policy for callers that want it (the editors
/// deliberately don't — a keyboard-only reviewer must not be surprised).
///
/// ponytail: ONE modal open at a time is the invariant (the palette + editors
/// open from exclusive signals; the trap root is the caller's own card class,
/// so even the remote combo keeps both traps scoped).
#[component]
pub(crate) fn Modal(
    label: String,
    trap: String,
    initial_focus: String,
    #[props(default)] close_on_overlay: bool,
    on_close: EventHandler<()>,
    children: Element,
) -> Element {
    use_effect(move || {
        let sel = initial_focus.clone();
        spawn(async move {
            let js = format!("const i=document.querySelector({sel:?}); if(i) i.focus();");
            let _ = document::eval(&js).await;
        });
    });
    rsx! {
        div {
            class: "fixed inset-0 z-50 bg-surface-overlay/80 flex items-center justify-center p-4",
            role: "dialog", "aria-modal": "true", "aria-label": "{label}",
            onmousedown: move |_| {
                if close_on_overlay {
                    on_close.call(());
                }
            },
            onkeydown: move |e: Event<KeyboardData>| {
                if e.key() == Key::Escape {
                    on_close.call(());
                } else if e.key() == Key::Tab {
                    let shift = e.modifiers().contains(Modifiers::SHIFT);
                    let t = trap.clone();
                    spawn(async move { focus_trap(&t, shift).await; });
                }
            },
            {children}
        }
    }
}

/// M2.2: the context drawer. Esc closes (clears the signal); Tab-cycling focus
/// trap (M7.3) wraps keyboard focus inside the dialog. The content is
/// already-fetched data pushed by a panel (no duplicate fetch).
#[component]
fn Drawer() -> Element {
    let mut ui = use_context::<UiState>();
    let content = (ui.drawer)();
    let detail = t("detail");
    let close_drawer = t("close_drawer");
    let nothing_selected = t("nothing_selected");
    rsx! {
        aside {
            class: "drawer",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": t("aria_detail_drawer"),
            tabindex: "0",
            onkeydown: move |e: Event<KeyboardData>| {
                // Esc closes; Tab / Shift+Tab cycle focus within the dialog.
                if e.key() == Key::Escape {
                    ui.drawer.set(None);
                } else if e.key() == Key::Tab {
                    let shift = e.modifiers().contains(Modifiers::SHIFT);
                    spawn(async move { focus_trap(".drawer", shift).await; });
                }
            },
            div { class: "flex justify-between items-center mb-3",
                h2 { class: "text-sm font-semibold text-muted-foreground", "{detail}" }
                button {
                    class: "btn btn-ghost btn-sm",
                    "aria-label": "{close_drawer}",
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
                None => rsx! { p { class: "text-ink-faint text-sm", "{nothing_selected}" } },
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

/// v1.17.6 M2.6: the landing route component.
#[component]
fn Overview() -> Element {
    overview::panel()
}

#[component]
fn Review() -> Element {
    review::panel()
}
/// v1.16.7 M1: the deep-linkable single proposal (`/review/:proposal_id`).
#[component]
fn ReviewDetail(proposal_id: i64) -> Element {
    review::detail(proposal_id)
}
#[component]
fn Recall() -> Element {
    recall::panel()
}
/// v1.17.7 M3: the knowledge-graph explore panel.
#[component]
fn Graph() -> Element {
    graph::panel()
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
/// v1.16.7 M1: the deep-linkable deletion certificate (`/subjects/certificate/:dsar_id`).
#[component]
fn DsarDetail(dsar_id: i64) -> Element {
    subjects::detail(dsar_id)
}
#[component]
fn Security() -> Element {
    security::panel()
}
#[component]
fn Audit(since: Option<String>, principal: Option<String>) -> Element {
    audit::panel(since, principal)
}
/// v1.17.7 M4: the create-workspace panel (ingest/procedures/consolidate).
#[component]
fn Create() -> Element {
    create::panel()
}
#[component]
fn Health() -> Element {
    health::panel()
}
/// v1.17.8 M5: Rights group — data panel (purge/export/retention/registries).
#[component]
fn Data() -> Element {
    data::panel()
}
/// v1.17.8 M6: Portability group — UMP wire ops panel.
#[component]
fn Ump() -> Element {
    ump::panel()
}
/// v1.17.8 M7: System group — console panel (domains/snapshot/art30/reindex/console).
#[component]
fn System() -> Element {
    system::panel()
}

/// v1.20.6 M1: Memory Operations — the live HITL work surface.
#[component]
fn Ops() -> Element {
    ops::panel()
}
#[component]
fn Register() -> Element {
    register::panel()
}
#[component]
fn Clients() -> Element {
    panels::console::panel()
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

    /// v1.27.20 M2.2: plaintext http over a NON-loopback address warns; every
    /// loopback form, https, and the empty default stay silent.
    #[test]
    fn plaintext_http_warns_non_loopback_only() {
        // The dev default + loopback forms are exempt (no nagging).
        assert!(!plaintext_http(""));
        assert!(!plaintext_http("http://127.0.0.1:8765"));
        assert!(!plaintext_http("http://127.0.0.10:8765"));
        assert!(!plaintext_http("http://localhost:8765"));
        assert!(!plaintext_http("http://[::1]:8765"));
        assert!(!plaintext_http("http://::1"));
        // https is exempt whatever the host.
        assert!(!plaintext_http("https://10.0.0.5:8765"));
        assert!(!plaintext_http("https://brain.example.com"));
        // The warning cases: scheme-http + non-loopback host.
        assert!(plaintext_http("http://10.0.0.5:8765"));
        assert!(plaintext_http("http://brain.lan"));
        assert!(plaintext_http("http://192.168.1.20:8765"));
    }

    /// v1.17.0 M3.4: the offline-prefill guard — a remembered base fills an
    /// EMPTY field but never overwrites what the operator typed.
    #[test]
    fn offline_prefill_fills_empty_field_only() {
        assert_eq!(
            prefill_if_empty("", "https://brain.example.com"),
            Some("https://brain.example.com".into())
        );
        assert_eq!(
            prefill_if_empty("  ", "https://brain.example.com"),
            Some("https://brain.example.com".into())
        );
        // A non-empty (even partial) field is left alone — the operator's input wins.
        assert_eq!(
            prefill_if_empty("https://other", "https://brain.example.com"),
            None
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

    /// v1.16.7 M5: the palette lists the six nav targets + (only when a client
    /// is configured) Sign out.
    #[test]
    fn palette_lists_nav_targets_and_conditional_signout() {
        let configured = palette_commands(true);
        let count: usize = configured
            .iter()
            .filter(|c| matches!(c, Command::Navigate(_)))
            .count();
        assert_eq!(
            count, 15,
            "the fifteen nav targets (Overview→Health + Ops/Data/UMP/System + Register + Clients)"
        );
        assert!(configured.iter().any(|c| matches!(c, Command::SignOut)));
        let anonymous = palette_commands(false);
        assert!(
            !anonymous.iter().any(|c| matches!(c, Command::SignOut)),
            "no sign-out for an unconfigured client"
        );
    }

    /// v1.16.7 M5 / v1.17.6 M1.2: grouped filtering is a case-insensitive
    /// substring match over keywords + labels; empty needle returns every
    /// group (per-group 5 cap), a no-match needle returns nothing.
    #[test]
    fn palette_filter_is_case_insensitive_substring() {
        let commands = palette_commands(true);
        let sec = palette_lookup(&commands, "SECURITY", &[]);
        let sec_flat: Vec<&Command> = sec.iter().flat_map(|(_, c)| c.iter()).collect();
        assert_eq!(sec_flat.len(), 1);
        assert_eq!(command_label(sec_flat[0]), "security_title");
        assert!(palette_lookup(&commands, "zzz-no-such", &[]).is_empty());
        let all = palette_lookup(&commands, "", &[]);
        let all_flat: Vec<&Command> = all.iter().flat_map(|(_, c)| c.iter()).collect();
        assert!(!all_flat.is_empty());
    }

    /// v1.17.6 M1.5: the palette's `Navigate` set is the single registration
    /// source of truth — every non-detail shell route must be reachable, or a
    /// new page ships without a palette entry. (Detail + Connect are excluded
    /// by design: they're deep links, not nav targets.)
    #[test]
    fn palette_navigate_covers_every_non_detail_route() {
        let nav: Vec<Route> = palette_commands(false)
            .into_iter()
            .filter_map(|c| match c {
                Command::Navigate(r) => Some(r),
                _ => None,
            })
            .collect();
        for r in [
            Route::Overview {},
            Route::Review {},
            Route::Recall {},
            Route::Graph {},
            Route::Subjects {},
            Route::Security {},
            Route::Audit {
                since: None,
                principal: None,
            },
            Route::Create {},
            Route::Health {},
            Route::Data {},
            Route::Ump {},
            Route::System {},
            Route::Ops {},
            Route::Register {},
            Route::Clients {},
        ] {
            assert!(nav.contains(&r), "palette missing Navigate for {r:?}");
        }
    }

    /// v1.20.3 "Classify" (G5) render boundary: the console strips invisible
    /// smuggling chars from displayed content but preserves visible Unicode.
    /// v1.27.21 (read-seam audit): the F-32 pins — U+061C (ALM) and the
    /// supplementary variation selectors — guard the copy's sync contract
    /// with the server lib (this copy had drifted behind exactly these two).
    #[test]
    fn strip_invisible_removes_smuggling_but_keeps_visible_text() {
        for c in [
            '\u{200B}',
            '\u{200C}',
            '\u{200D}',
            '\u{2060}',
            '\u{FEFF}',
            '\u{00AD}',
            '\u{E0000}',
            '\u{FE00}',
            // Bidi controls: LRM, RLO (override), LRI (isolate), ALM (F-32).
            '\u{200E}',
            '\u{202E}',
            '\u{2066}',
            '\u{061C}',
            // Supplementary variation selectors (F-32): both range ends.
            '\u{E0100}',
            '\u{E01EF}',
        ] {
            assert_eq!(
                strip_invisible(&format!("ig{}nore", c)),
                "ignore".to_string()
            );
        }
        assert_eq!(
            strip_invisible("héllo wörld 日本語"),
            "héllo wörld 日本語".to_string()
        );
        assert_eq!(strip_invisible(""), "");
    }
}
