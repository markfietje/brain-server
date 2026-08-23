//! Panel modules. Recall + Health are fully wired to live endpoints here in
//! the scaffold; the other four flesh out as their v1.14/v1.15 APIs ship
//! (proposals → v1.14.0, DSAR/tombstones → v1.15.0, quarantine/audit-verify
//! already exist, recall-trace → v1.15.0).

pub mod audit;
pub mod console;
pub mod consolidate;
/// v1.28.19 Witness: the /runs/:run_id conversation surface — the run
/// transcript (assembler-fed nodes), the AskHuman answer card, steering.
pub mod conversation;
pub mod create;
pub mod data;
pub mod graph;
pub mod health;
pub mod ingest;
pub mod ops;
pub mod overview;
pub mod procedures;
pub mod recall;
pub mod register;
pub mod review;
pub mod security;
pub mod subjects;
pub mod system;

/// v1.20.3/1.20.6: a `screen_verdict` (`clean`/`quarantine`) maps to a
/// semantic token + a human label. Shared by the review detail card and the
/// /ops flagged surface so one source of truth drives both. v1.27.20 M3: the
/// label is now an i18n KEY (`verdict_clean`/`verdict_quarantined`) — the
/// render sites resolve it via `t()` so every locale speaks it.
pub fn verdict_badge(v: &str) -> &'static str {
    match v {
        "quarantine" => "warn",
        _ => "ok",
    }
}

pub fn verdict_label(v: &str) -> &'static str {
    match v {
        "quarantine" => "verdict_quarantined",
        _ => "verdict_clean",
    }
}

/// v1.20.14 "Steer" M1: a proposal whose `edited_at` is set was rewritten in
/// place before decision — the reviewer (and a later auditor) should see that
/// the content shown is not the original capture. Renders a `warn` badge with a
/// stable label; `None` (never edited) renders nothing. v1.27.20 M3: the label
/// is an i18n KEY (`edited`) — render sites resolve it via `t()`.
pub fn edited_label(edited_at: Option<i64>) -> Option<&'static str> {
    edited_at.map(|_| "edited")
}
pub mod ump;

use dioxus::prelude::*;

/// v1.16.3 M1.1: set the document title reactively on the web. Runs once per
/// panel mount; `document::eval` is the cross-platform primitive (no-op where
/// there's no DOM — desktop/mobile set their own window title).
pub fn use_document_title(title: impl Fn() -> String + 'static) {
    use_effect(move || {
        // The title is a static literal per panel; JSON-encode it so the
        // eval string can never break out of the JS assignment.
        let js = format!(
            "document.title = {}",
            serde_json::to_string(&title()).unwrap_or_default()
        );
        let _ = document::eval(&js);
    });
}

/// v1.16.3 M1.2: the panel's main heading. `tabindex="-1"` makes it
/// programmatically focusable but not in tab order; on mount focus moves here
/// so screen-reader users get a signal that the view changed (SPA focus
/// management, WAI-ARIA Authoring Practices). `scroll-margin-top` in input.css
/// keeps it clear of the sticky nav (WCAG 2.4.11). `set_focus` is cancel-safe:
/// if the panel unmounts before the async focus lands, the element is gone and
/// the call is a no-op.
#[component]
pub fn PageTitle(children: Element) -> Element {
    rsx! {
        h1 {
            tabindex: "-1",
            onmounted: move |el| {
                let el = el.data();
                spawn(async move {
                    let _ = el.set_focus(true).await;
                });
            },
            {children}
        }
    }
}

/// v1.17.0 M2.4: a portable refresh control. The queue/list panels (Review,
/// Audit, Health) re-fetch when their `refresh` signal bumps; this button is
/// that trigger, exposed for the touch-first crowd. A native pull-to-refresh
/// *gesture* is platform glue (needs touch events — the v1.18.0 pass); the
/// button is the honest, testable equivalent that works on every renderer.
#[component]
pub fn RefreshButton(mut refresh: Signal<u32>) -> Element {
    rsx! {
        button {
            class: "btn btn-ghost btn-md",
            "aria-label": "refresh",
            title: "refresh",
            onclick: move |_| refresh += 1,
            "⟳"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{verdict_badge, verdict_label};

    #[test]
    fn verdict_maps_quarantine_to_warn_and_clean_to_ok() {
        assert_eq!(verdict_badge("quarantine"), "warn");
        assert_eq!(verdict_label("quarantine"), "verdict_quarantined");
        assert_eq!(verdict_badge("clean"), "ok");
        assert_eq!(verdict_label("clean"), "verdict_clean");
        assert_eq!(verdict_badge("reject"), "ok"); // reject never persists; reads as quarantine is handled server-side
        assert_eq!(verdict_badge(""), "ok"); // unknown/legacy -> clean posture
    }

    /// The verdict label is an i18n KEY and must resolve in the default
    /// locale — the badges speak the operator's language, never raw English.
    /// (The value may legitimately equal the key — "edited" is both.)
    #[test]
    fn verdict_and_edited_labels_resolve_as_keys() {
        for k in ["verdict_clean", "verdict_quarantined", "edited"] {
            let resolved = crate::i18n::resolve_fmt(k, "en", &[]);
            assert!(!resolved.is_empty(), "{k} must resolve in en");
        }
    }
}
