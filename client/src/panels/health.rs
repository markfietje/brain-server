//! Health panel — the connect-first probe + capacity story (DESIGN §4.5).
//! One resource, rendered as the capacity envelope (docs/RSS vs cap).

use crate::api::{error_message, ApiClient};
use crate::panels::{use_document_title, PageTitle};
use crate::UiState;
use dioxus::prelude::*;

pub fn panel() -> Element {
    use_document_title(|| "Health — brain".into());
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
        PageTitle { {crate::i18n::t("health_title")} }
        div { class: "mt-2 grid gap-4 md:grid-cols-2",
            match &*health.read() {
                Some(Ok(h)) => rsx! {
                    div { class: "card",
                        div { class: "card-header", div { class: "card-title", "Service" } }
                        dl { class: "card-body grid grid-cols-2 gap-x-4 gap-y-1.5 text-sm tabular",
                            dt { class: "text-muted-foreground", "status" }  dd { "{h.status}" }
                            dt { class: "text-muted-foreground", "version" } dd { "{h.version}" }
                            if let Some(c) = &h.capacity {
                                dt { class: "text-muted-foreground", "docs" }     dd { "{c.docs} / {c.max_docs}" }
                                dt { class: "text-muted-foreground", "rss" }      dd { "{c.rss_mib} / {c.max_rss_mib} MiB" }
                                dt { class: "text-muted-foreground", "capacity" } dd { span { class: cap_class(&c.status), "{c.status}" } }
                            } else {
                                dt { class: "text-muted-foreground", "capacity" } dd { "unavailable" }
                            }
                            if let Some(h2) = &h.hardening {
                                dt { class: "text-muted-foreground", "unsafe blocks" } dd { "{h2.unsafe_blocks}" }
                                dt { class: "text-muted-foreground", "panics caught" } dd { "{h2.panics_caught}" }
                            }
                        }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "text-danger", "health failed: {error_message(&e)}" } },
                None => rsx! { p { class: "text-muted-foreground", "…" } },
            }
            match &*stats.read() {
                Some(Ok(s)) => rsx! {
                    div { class: "card",
                        div { class: "card-header", div { class: "card-title", "Corpus" } }
                        dl { class: "card-body grid grid-cols-2 gap-x-4 gap-y-1.5 text-sm tabular",
                            dt { class: "text-muted-foreground", "chunks" }        dd { "{s.count}" }
                            dt { class: "text-muted-foreground", "embeddings" }    dd { "{s.embeddings}" }
                            dt { class: "text-muted-foreground", "entities" }      dd { "{s.entities}" }
                            dt { class: "text-muted-foreground", "relationships" } dd { "{s.relationships}" }
                            dt { class: "text-muted-foreground", "model" }         dd { "{s.model}" }
                        }
                    }
                },
                Some(Err(_)) => rsx! {}, // stats is a bonus; health carries the panel
                None => rsx! {},
            }
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
