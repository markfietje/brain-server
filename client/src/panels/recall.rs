//! Recall inspector — the decision-path viewer (DESIGN §4.2).
//! The idiomatic Dioxus pattern: a `use_signal` for the query → a `use_resource`
//! that subscribes to it and auto-refetches on change (cancelling in-flight).

use crate::api::{ApiClient, Hit};
use dioxus::prelude::*;

pub fn panel() -> Element {
    // In the full app this is provided at the root after connect-first onboarding.
    // For the scaffold, default to the loopback brain-server.
    let api = use_context::<Signal<ApiClient>>();
    let query = use_signal(String::new);

    // use_resource subscribes to `query` (reads it) → reruns on change.
    let recall = use_resource(move || {
        let q = query();
        let api = api();
        async move { api.recall(&q).await }
    });

    rsx! {
        h1 { "Recall inspector" }
        input {
            class: "border rounded px-2 py-1 w-full",
            placeholder: "query brain-server (min 5 chars)…",
            value: "{query}",
            oninput: move |e| query.set(e.value()),
        }
        match &*recall.read() {
            Some(Ok(hits)) if !hits.is_empty() => rsx! {
                ul { class: "mt-2 divide-y",
                    for h in hits { HitRow { hit: h.clone() } }
                }
            },
            Some(Ok(_)) => rsx! { p { class: "text-gray-500 mt-2", "no hits" } },
            Some(Err(e)) => rsx! { p { class: "text-red-600 mt-2", "recall failed: {e:?}" } },
            None => rsx! { p { class: "text-gray-500 mt-2", "…" } },
        }
    }
}

#[component]
fn HitRow(hit: Hit) -> Element {
    rsx! {
        li { class: "py-2",
            div { class: "flex justify-between",
                span { "chunk #{hit.id}" }
                span { class: "font-mono text-sm", "score {hit.score:.3}" }
            }
            p { class: "text-sm text-gray-700", "{hit.content}" }
        }
    }
}
