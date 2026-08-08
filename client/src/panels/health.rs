//! Health panel — the connect-first probe + capacity story (DESIGN §4.5).
//! One resource, rendered as the capacity envelope (docs/RSS vs cap).

use crate::api::ApiClient;
use dioxus::prelude::*;

pub fn panel() -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let health = use_resource(move || {
        let api = api();
        async move { api.health().await }
    });

    rsx! {
        h1 { "Health" }
        match &*health.read() {
            Some(Ok(h)) => rsx! {
                dl { class: "mt-2 grid grid-cols-2 gap-1 text-sm",
                    dt { "status" }      dd { "{h.status}" }
                    dt { "version" }     dd { "{h.version}" }
                    dt { "docs" }        dd { "{h.capacity.docs} / {h.capacity.max_docs}" }
                    dt { "rss" }         dd { "{h.capacity.rss_mib} / {h.capacity.max_rss_mib} MiB" }
                    dt { "capacity" }    dd { "{h.capacity.status}" }
                }
            },
            Some(Err(e)) => rsx! { p { class: "text-red-600", "health failed: {e:?}" } },
            None => rsx! { p { class: "text-gray-500", "…" } },
        }
    }
}
