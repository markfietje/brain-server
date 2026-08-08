//! brain-client — the Dioxus control surface for brain-server.
//!
//! One codebase → web (WASM/DOM) + desktop + iOS + Android.
//! Run:  `dx serve --platform web`  (or desktop / ios / android)
//!
//! Architecture (see DESIGN_v1.16.0_Client.md):
//!   Router::<Route> → AppShell (nav rail + topbar + Outlet) → panels.
//!   Root context provides an ApiClient (bearer attached) the panels read via
//!   `use_resource`, which auto-refetches on signal change.

// The #[component] macro nests the fn body in an extra block, so one-line
// route components trip unused_braces — a macro artifact, not our code.
#![allow(unused_braces)]

use dioxus::prelude::*;
use panels::{audit, health, recall, review, security, subjects};

mod api;
mod panels;

use api::ApiClient;

/// The six task-oriented routes (DESIGN §2). Deep-linkable: a recall trace, a
/// DSAR certificate, a specific proposal are all URL-addressable.
#[derive(Clone, Debug, PartialEq, Routable)]
enum Route {
    #[layout(AppShell)]
    #[route("/")]
    Review {},
    #[route("/recall")]
    Recall {},
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
    // Scaffold default: loopback brain-server, no token. The connect-first
    // onboarding screen (DESIGN §3) replaces this with interactive URL+token.
    let _ = use_context_provider(|| Signal::new(ApiClient::new("http://127.0.0.1:8765", None)));
    rsx! { Router::<Route> {} }
}

/// AppShell — the nav rail (desktop/web) that persists across route transitions.
/// On mobile (v1.17.0) the same routes render under a bottom tab bar; the
/// responsive swap lives here, gated on viewport width.
#[component]
fn AppShell() -> Element {
    rsx! {
        nav {
            class: "flex gap-2 p-2 border-b",
            Link { to: Route::Review {}, active_class: "font-bold", "Review" }
            Link { to: Route::Recall {}, active_class: "font-bold", "Recall" }
            Link { to: Route::Subjects {}, active_class: "font-bold", "Subjects" }
            Link { to: Route::Security {}, active_class: "font-bold", "Security" }
            Link { to: Route::Audit {}, active_class: "font-bold", "Audit" }
            Link { to: Route::Health {}, active_class: "font-bold", "Health" }
        }
        main { class: "p-4", Outlet::<Route> {} }
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
