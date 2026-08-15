//! Health panel — the connect-first probe + capacity story (DESIGN §4.5).
//! One resource, rendered as the capacity envelope (docs/RSS vs cap).
//! v1.21.0 "Profiles" M4: a third card — the home domain's active profile
//! + effective knobs (transparency = the 2026 compliance ask).

use crate::api::{error_message, ApiClient};
use crate::panels::{use_document_title, PageTitle, RefreshButton};
use crate::UiState;
use dioxus::prelude::*;

pub fn panel() -> Element {
    use_document_title(|| "Health — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let _ui = use_context::<UiState>();
    // v1.17.0 M2.4: a portable refresh trigger. Health reads live endpoints
    // (probe + stats) whose numbers change on other panels' writes; the
    // button bumps the signal the resources subscribe to.
    let refresh = use_signal(|| 0u32);
    let health = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().health().await }
    });
    let stats = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().stats().await }
    });
    // v1.21.0 "Profiles": the home domain's binding + knobs. An unreachable /
    // older server degrades to the unbound card (never an error — the probe +
    // corpus cards stay authoritative).
    let profile = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().domain_profile("global").await.ok() }
    });

    // Hoisted reads + labels so the rsx stays statement-free (a `let` as a
    // direct child of an rsx `if`/`match` arm breaks the parser — the
    // panels/data.rs precedent).
    let prof_title = crate::i18n::t("health_profile");
    let prof_none = crate::i18n::t("health_profile_none");
    let prof_knobs_note = crate::i18n::t("health_profile_knobs");
    let prof_loaded = profile.read().clone().flatten();
    let prof_domain = prof_loaded
        .as_ref()
        .map(|d| d.domain.clone())
        .unwrap_or_else(|| "global".to_string());
    let prof_name_line = prof_loaded
        .as_ref()
        .and_then(|d| d.profile.clone())
        .unwrap_or_else(|| prof_none.clone());
    let knobs = prof_loaded.as_ref().and_then(|d| d.knobs.clone());
    let prof_scope = knobs.as_ref().and_then(|k| k.default_access_scope.clone());
    let prof_pii = knobs.as_ref().and_then(|k| k.pii_mode.clone());
    let prof_retention = knobs.as_ref().and_then(|k| k.retention_label());
    let prof_audit = knobs.as_ref().and_then(|k| k.audit_level.clone());
    let prof_kinds = knobs
        .as_ref()
        .and_then(|k| k.kinds.clone())
        .map(|k| k.join(", "));
    let prof_hold = knobs.as_ref().and_then(|k| k.legal_hold_default);

    rsx! {
        PageTitle { {crate::i18n::t("health_title")} }
        div { class: "flex justify-end my-2", RefreshButton { refresh } }
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
            // v1.21.0 "Profiles" M4: the active profile + effective knobs —
            // the transparency card. Unbound shows the server-default posture
            // explicitly, not a blank (the row-wins note is the honest frame).
            div { class: "card",
                div { class: "card-header",
                    div { class: "card-title", "{prof_title}" }
                    span { class: "badge", "{prof_domain}" }
                }
                dl { class: "card-body grid grid-cols-2 gap-x-4 gap-y-1.5 text-sm tabular",
                    dt { class: "text-muted-foreground", "profile" } dd { "{prof_name_line}" }
                    if let Some(v) = &prof_scope {
                        dt { class: "text-muted-foreground", "default scope" } dd { "{v}" }
                    }
                    if let Some(v) = &prof_pii {
                        dt { class: "text-muted-foreground", "pii mode" } dd { "{v}" }
                    }
                    if let Some(v) = &prof_retention {
                        dt { class: "text-muted-foreground", "retention" } dd { "{v}" }
                    }
                    if let Some(v) = &prof_audit {
                        dt { class: "text-muted-foreground", "audit level" } dd { "{v}" }
                    }
                    if let Some(v) = &prof_kinds {
                        dt { class: "text-muted-foreground", "kinds" } dd { "{v}" }
                    }
                    if let Some(v) = prof_hold {
                        dt { class: "text-muted-foreground", "legal hold default" } dd { "{v}" }
                    }
                    dt { class: "text-muted-foreground", "note" } dd { "{prof_knobs_note}" }
                }
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
