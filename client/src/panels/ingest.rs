//! Create workspace: Ingest (v1.17.7 M4.1, Write group). Three tabs —
//! Structured memory (`/ingest`), Markdown paste (`/ingest/markdown`), Memory
//! batch (`/ingest/memory`) — each a form → an honest result summary via the
//! `parse_ingest_result` core (created / duplicate / error, no silent failures).
//!
//! The entities/relations editor is a free-text JSON textarea; the server is
//! the source of truth for shape. The panel validates JSON before send so a
//! typo surfaces as a readable error, not a 400.

use crate::api::{parse_ingest_result, ApiClient, IngestOutcome};
use dioxus::prelude::*;

pub fn panel() -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<crate::UiState>();
    let writes = (ui.writes_enabled)();

    let tab = use_signal(|| "structured".to_string());
    // Structured form fields.
    let mut s_title = use_signal(String::new);
    let mut s_content = use_signal(String::new);
    let mut s_kind = use_signal(String::new);
    let mut s_domain = use_signal(String::new);
    let mut s_entities = use_signal(String::new);
    let mut s_relations = use_signal(String::new);
    // Markdown form fields.
    let mut md_content = use_signal(String::new);
    let mut md_title = use_signal(String::new);
    let mut md_path = use_signal(String::new);
    let mut md_domain = use_signal(String::new);
    let mut md_replace = use_signal(|| false);
    // Memory form fields.
    let mut mem_content = use_signal(String::new);
    // Result: `Some(Ok(outcome))` success, `Some(Err(msg))` failure, `None` idle.
    let mut result = use_signal(|| None::<Result<IngestOutcome, String>>);

    let structured_tab = crate::i18n::t("ingest_tab_structured");
    let markdown_tab = crate::i18n::t("ingest_tab_markdown");
    let memory_tab = crate::i18n::t("ingest_tab_memory");
    let content_lbl = crate::i18n::t("ingest_content");
    let title_lbl = crate::i18n::t("ingest_title");
    let kind_lbl = crate::i18n::t("ingest_kind");
    let domain_lbl = crate::i18n::t("ingest_domain");
    let entities_lbl = crate::i18n::t("ingest_entities");
    let relations_lbl = crate::i18n::t("ingest_relations");
    let source_path_lbl = crate::i18n::t("ingest_source_path");
    let replace_lbl = crate::i18n::t("ingest_replace");
    let submit_lbl = crate::i18n::t("ingest_submit");
    let bad_json = crate::i18n::t("ingest_bad_json");
    let mem_hint = crate::i18n::t("ingest_mem_hint");

    // Submit helpers. Each validates, then spawns the write; the result signal
    // drives the aria-live summary. Mutations respect the writes gate.
    let run_submit = move |_| match tab().as_str() {
        "structured" => {
            let json_err = if s_entities().trim().is_empty() || s_relations().trim().is_empty() {
                None
            } else {
                let e = serde_json::from_str::<serde_json::Value>(&s_entities());
                let r = serde_json::from_str::<serde_json::Value>(&s_relations());
                if e.is_err() || r.is_err() {
                    Some(bad_json.clone())
                } else {
                    None
                }
            };
            if let Some(m) = json_err {
                result.set(Some(Err(m)));
                return;
            }
            let api = api;
            let s_title = s_title();
            let s_content = s_content();
            let s_kind = s_kind();
            let s_domain = s_domain();
            let ents = serde_json::from_str::<serde_json::Value>(&s_entities())
                .unwrap_or(serde_json::json!([]));
            let rels = serde_json::from_str::<serde_json::Value>(&s_relations())
                .unwrap_or(serde_json::json!([]));
            spawn(async move {
                let out = match api()
                    .ingest_structured(&s_title, &s_content, &s_kind, &s_domain, &ents, &rels)
                    .await
                {
                    Ok(json) => Ok(parse_ingest_result(&json)),
                    Err(e) => Err(crate::api::error_message(&e)),
                };
                result.set(Some(out));
            });
        }
        "markdown" => {
            let api = api;
            let c = md_content();
            let t = md_title();
            let p = md_path();
            let d = md_domain();
            let rep = md_replace();
            spawn(async move {
                let out = match api()
                    .ingest_markdown(
                        &c,
                        if t.trim().is_empty() { None } else { Some(&t) },
                        if p.trim().is_empty() { None } else { Some(&p) },
                        &d,
                        rep,
                    )
                    .await
                {
                    Ok(json) => Ok(parse_ingest_result(&json)),
                    Err(e) => Err(crate::api::error_message(&e)),
                };
                result.set(Some(out));
            });
        }
        _ => {
            let api = api;
            let c = mem_content();
            spawn(async move {
                let out = match api().ingest_memory(&c).await {
                    Ok(json) => Ok(parse_ingest_result(&json)),
                    Err(e) => Err(crate::api::error_message(&e)),
                };
                result.set(Some(out));
            });
        }
    };

    rsx! {
        div { class: "flex gap-2 my-2",
            { tab_btn(tab, "structured", &structured_tab) }
            { tab_btn(tab, "markdown", &markdown_tab) }
            { tab_btn(tab, "memory", &memory_tab) }
        }
        div { class: "card",
            div { class: "card-body",
                match tab().as_str() {
                    "structured" => rsx! {
                        label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                            "{title_lbl}"
                            input { class: "input", value: "{s_title}", oninput: move |e| s_title.set(e.value()), "aria-label": "{title_lbl}" }
                        }
                        label { class: "flex flex-col gap-1 text-xs text-muted-foreground mt-3",
                            "{content_lbl}"
                            textarea { class: "input min-h-32", value: "{s_content}", oninput: move |e| s_content.set(e.value()), "aria-label": "{content_lbl}" }
                        }
                        div { class: "grid gap-3 md:grid-cols-2 mt-3",
                            label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                                "{kind_lbl}"
                                input { class: "input", placeholder: "fact · procedure · step · decision", value: "{s_kind}", oninput: move |e| s_kind.set(e.value()), "aria-label": "{kind_lbl}" }
                            }
                            label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                                "{domain_lbl}"
                                input { class: "input", placeholder: "global", value: "{s_domain}", oninput: move |e| s_domain.set(e.value()), "aria-label": "{domain_lbl}" }
                            }
                        }
                        label { class: "flex flex-col gap-1 text-xs text-muted-foreground mt-3",
                            "{entities_lbl}"
                            textarea { class: "input min-h-20 font-mono", placeholder: {r#"[{"name":"acme","type":"company"}]"#}, value: "{s_entities}", oninput: move |e| s_entities.set(e.value()), "aria-label": "{entities_lbl}" }
                        }
                        label { class: "flex flex-col gap-1 text-xs text-muted-foreground mt-3",
                            "{relations_lbl}"
                            textarea { class: "input min-h-20 font-mono", placeholder: {r#"[{"from":"dave","to":"acme","type":"works_at"}]"#}, value: "{s_relations}", oninput: move |e| s_relations.set(e.value()), "aria-label": "{relations_lbl}" }
                        }
                    },
                    "markdown" => rsx! {
                        label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                            "{content_lbl}"
                            textarea { class: "input min-h-40 font-mono", value: "{md_content}", oninput: move |e| md_content.set(e.value()), "aria-label": "{content_lbl}" }
                        }
                        div { class: "grid gap-3 md:grid-cols-2 mt-3",
                            label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                                "{title_lbl}"
                                input { class: "input", value: "{md_title}", oninput: move |e| md_title.set(e.value()), "aria-label": "{title_lbl}" }
                            }
                            label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                                "{source_path_lbl}"
                                input { class: "input", value: "{md_path}", oninput: move |e| md_path.set(e.value()), "aria-label": "{source_path_lbl}" }
                            }
                            label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                                "{domain_lbl}"
                                input { class: "input", placeholder: "global", value: "{md_domain}", oninput: move |e| md_domain.set(e.value()), "aria-label": "{domain_lbl}" }
                            }
                            label { class: "flex items-center gap-1.5 text-sm",
                                input { "type": "checkbox", class: "accent-accent", checked: md_replace(), onchange: move |e| md_replace.set(e.value() == "true") }
                                "{replace_lbl}"
                            }
                        }
                    },
                    _ => rsx! {
                        p { class: "text-xs text-muted-foreground", "{mem_hint}" }
                        textarea { class: "input min-h-40 font-mono mt-2", value: "{mem_content}", oninput: move |e| mem_content.set(e.value()), "aria-label": "{memory_tab}" }
                    }
                }
                div { class: "mt-4 flex items-center gap-3",
                    button {
                        class: "btn btn-primary",
                        disabled: !writes,
                        onclick: run_submit,
                        "{submit_lbl}"
                    }
                    div { "role": "status", "aria-live": "polite", class: "text-sm",
                        { result_view(&result) }
                    }
                }
            }
        }
    }
}

/// The tab button (real `<button>`, aria-pressed state).
fn tab_btn(tab: Signal<String>, id: &'static str, label: &str) -> Element {
    let active = tab() == id;
    rsx! {
        button {
            class: if active { "btn btn-secondary btn-sm" } else { "btn btn-ghost btn-sm" },
            "aria-pressed": active,
            onclick: move |_| {
                let mut t = tab;
                t.set(id.to_string())
            },
            "{label}"
        }
    }
}

fn result_view(result: &Signal<Option<Result<IngestOutcome, String>>>) -> Element {
    match result() {
        Some(Ok(IngestOutcome::Created)) => rsx! {
            span { class: "text-ok", {crate::i18n::t("outcome_created")} }
        },
        Some(Ok(IngestOutcome::Duplicate)) => rsx! {
            span { class: "text-muted-foreground", {crate::i18n::t("outcome_duplicate")} }
        },
        Some(Ok(IngestOutcome::Error(m))) => rsx! {
            span { class: "text-danger", "{m}" }
        },
        Some(Err(m)) => rsx! { span { class: "text-danger", "{m}" } },
        None => rsx! { span { class: "text-muted-foreground", "…" } },
    }
}
