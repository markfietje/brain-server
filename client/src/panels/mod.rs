//! Panel modules. Recall + Health are fully wired to live endpoints here in
//! the scaffold; the other four flesh out as their v1.14/v1.15 APIs ship
//! (proposals → v1.14.0, DSAR/tombstones → v1.15.0, quarantine/audit-verify
//! already exist, recall-trace → v1.15.0).

pub mod audit;
pub mod health;
pub mod recall;
pub mod review;
pub mod security;
pub mod subjects;

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
