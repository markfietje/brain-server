//! Health panel — the connect-first probe + capacity story (DESIGN §4.5).
//! One resource, rendered as the capacity envelope (docs/RSS vs cap).

use crate::api::ApiClient;
use crate::UiState;
use dioxus::prelude::*;

pub fn panel() -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let _ui = use_context::<UiState>();
    let health = use_resource(move || {
        let api = api;
        async move { api().health().await }
    });
    let stats = use_resource(move || {
        let api = api;
        async move { api().stats().await }
    });

    rsx! {
        h1 { "Health" }
        match &*health.read() {
            Some(Ok(h)) => rsx! {
                dl { class: "mt-2 grid grid-cols-2 gap-1 text-sm tabular",
                    dt { class: "text-ink-muted", "status" }  dd { "{h.status}" }
                    dt { class: "text-ink-muted", "version" } dd { "{h.version}" }
                    if let Some(c) = &h.capacity {
                        dt { class: "text-ink-muted", "docs" }     dd { "{c.docs} / {c.max_docs}" }
                        dt { class: "text-ink-muted", "rss" }      dd { "{c.rss_mib} / {c.max_rss_mib} MiB" }
                        dt { class: "text-ink-muted", "capacity" } dd { span { class: cap_class(&c.status), "{c.status}" } }
                    } else {
                        dt { class: "text-ink-muted", "capacity" } dd { "unavailable" }
                    }
                    if let Some(h2) = &h.hardening {
                        dt { class: "text-ink-muted", "unsafe blocks" } dd { "{h2.unsafe_blocks}" }
                        dt { class: "text-ink-muted", "panics caught" } dd { "{h2.panics_caught}" }
                    }
                }
            },
            Some(Err(e)) => rsx! { p { class: "text-danger", "health failed: {e}" } },
            None => rsx! { p { class: "text-ink-muted", "…" } },
        }
        match &*stats.read() {
            Some(Ok(s)) => rsx! {
                h2 { class: "text-lg mt-4", "Corpus" }
                dl { class: "grid grid-cols-2 gap-1 text-sm tabular",
                    dt { class: "text-ink-muted", "chunks" }        dd { "{s.count}" }
                    dt { class: "text-ink-muted", "embeddings" }    dd { "{s.embeddings}" }
                    dt { class: "text-ink-muted", "entities" }      dd { "{s.entities}" }
                    dt { class: "text-ink-muted", "relationships" } dd { "{s.relationships}" }
                    dt { class: "text-ink-muted", "model" }         dd { "{s.model}" }
                }
            },
            Some(Err(_)) => rsx! {}, // stats is a bonus; health carries the panel
            None => rsx! {},
        }
    }
}

fn cap_class(status: &str) -> &'static str {
    match status {
        "ok" => "text-ok",
        "warning" => "text-warn",
        "exceeded" => "text-danger",
        _ => "",
    }
}
