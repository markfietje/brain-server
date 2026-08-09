//! Graph — browse + traverse the knowledge graph (v1.17.7 M3, Explore group).
//! Two surfaces: an entity search card (type + typed relation rows) and a
//! bounded traverse view that renders the v1.7.0 `paths` chains faithfully
//! (`A --rel--> B`) with the flat `traversal` rows as the detail table.
//!
//! The wire decode + chain text live in `crate::api` (`parse_entity` /
//! `parse_traverse` / `render_path`) as pure cores so the tests pin the field
//! names the UI reads. This file is plumbing: signal wiring + rendering.

use crate::api::{kind_is_valid, parse_entity, render_path, ApiClient, ApiError};
use crate::panels::{use_document_title, PageTitle};
use dioxus::prelude::*;

/// v1.17.7 M3.3: the traverse controls, held as one struct so the resource
/// subscription + the submit button share a single signal (no field-by-field
/// re-fetch). `None` = no traverse requested yet.
#[derive(Clone, Debug, PartialEq)]
struct TraverseReq {
    start: String,
    depth: u8,
    kind: String,
    at: String,
    cross_domain: bool,
}

/// v1.17.7 M3.2: the debounce window (mirrors the recall panel).
const DEBOUNCE_MS: u64 = 300;

pub fn panel() -> Element {
    use_document_title(|| "Graph — brain".into());
    let api = use_context::<Signal<ApiClient>>();

    // Entity search (debounced, the recall pattern).
    let mut ent_input = use_signal(String::new);
    let mut ent_query = use_signal(String::new);
    let mut ent_gen = use_signal(|| 0u64);

    let ent_oninput = move |e: Event<FormData>| {
        ent_input.set(e.value());
        ent_gen += 1;
        let g = ent_gen();
        let val = ent_input();
        spawn(async move {
            let _ = document::eval(&format!(
                "return await new Promise(r => setTimeout(r, {DEBOUNCE_MS}));"
            ))
            .await;
            if g == ent_gen() {
                ent_query.set(val);
            }
        });
    };

    let entity = use_resource(move || {
        let q = ent_query();
        async move {
            if q.trim().is_empty() {
                return Err(ApiError::Status(400, "empty".into()));
            }
            api().graph_entity(&q).await
        }
    });

    // Traverse controls + submit.
    let mut req = use_signal(|| None::<TraverseReq>);
    let mut t_start = use_signal(String::new);
    let mut t_depth = use_signal(|| "2".to_string());
    let mut t_kind = use_signal(String::new);
    let mut t_at = use_signal(String::new);
    let mut t_cross = use_signal(|| false);

    let traverse = use_resource(move || {
        let req = req();
        async move {
            let r = req
                .as_ref()
                .ok_or(ApiError::Status(400, "no query".into()))?;
            api()
                .graph_traverse(&r.start, r.depth, &r.kind, &r.at, r.cross_domain)
                .await
        }
    });

    // i18n strings (precomputed — never nest a `t()` in a formatted string).
    let entity_ph = crate::i18n::t("graph_entity_ph");
    let browse = crate::i18n::t("graph_browse");
    let type_lbl = crate::i18n::t("graph_type");
    let relations_lbl = crate::i18n::t("graph_relations");
    let traverse_title = crate::i18n::t("graph_traverse");
    let start_lbl = crate::i18n::t("graph_start");
    let depth_lbl = crate::i18n::t("graph_depth");
    let kind_lbl = crate::i18n::t("graph_kind");
    let at_lbl = crate::i18n::t("graph_at");
    let cross_lbl = crate::i18n::t("graph_cross_domain");
    let run_lbl = crate::i18n::t("graph_run");
    let rel_lbl = crate::i18n::t("graph_rel");
    let out_lbl = crate::i18n::t("graph_out");
    let in_lbl = crate::i18n::t("graph_in");
    let no_entity = crate::i18n::t("graph_no_entity");
    let paths_lbl = crate::i18n::t("graph_paths");
    let rows_lbl = crate::i18n::t("graph_rows");
    let none = crate::i18n::t("none");

    rsx! {
        PageTitle { {crate::i18n::t("graph_title")} }
        // --- Entity browse ---
        div { class: "card",
            div { class: "card-header", div { class: "card-title", "{browse}" } }
            div { class: "card-body",
                input {
                    class: "input w-full",
                    placeholder: "{entity_ph}",
                    value: "{ent_input}",
                    oninput: ent_oninput,
                    "aria-label": "{entity_ph}",
                }
                div { class: "mt-3",
                    match &*entity.read() {
                        Some(Ok(raw)) => match parse_entity(raw) {
                            Some(ev) => rsx! {
                                div { class: "flex items-center gap-2",
                                    span { class: "font-semibold", "{ev.name}" }
                                    span { class: "badge badge-neutral", "{type_lbl}: {ev.entity_type}" }
                                }
                                if ev.relations.is_empty() {
                                    p { class: "text-sm text-muted-foreground mt-2", "{none}" }
                                } else {
                                    div { class: "overflow-x-auto mt-2",
                                        table { class: "table",
                                            thead { tr {
                                                th { "{rel_lbl}" } th { "{relations_lbl}" } th { "{type_lbl}" }
                                            } }
                                            tbody {
                                                for rel in &ev.relations {
                                                    tr {
                                                        td { class: "font-mono text-xs", "{rel.relation_type}" }
                                                        td { "{rel.other}" }
                                                        td {
                                                            span { class: if rel.dir == "out" { "badge badge-info" } else { "badge badge-neutral" },
                                                                if rel.dir == "out" { "{out_lbl}" } else { "{in_lbl}" }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            None => rsx! { p { class: "text-danger text-sm", "{no_entity}" } },
                        },
                        Some(Err(e)) => rsx! { p { class: "text-danger text-sm", "{crate::api::error_message(&e)}" } },
                        None => rsx! { p { class: "text-muted-foreground text-sm", "…" } },
                    }
                }
            }
        }
        // --- Traverse ---
        div { class: "card mt-4",
            div { class: "card-header", div { class: "card-title", "{traverse_title}" } }
            div { class: "card-body",
                div { class: "grid gap-3 md:grid-cols-2",
                    label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                        "{start_lbl}"
                        input {
                            class: "input",
                            value: "{t_start}",
                            oninput: move |e| t_start.set(e.value()),
                            "aria-label": "{start_lbl}",
                        }
                    }
                    label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                        "{depth_lbl}"
                        input {
                            class: "input",
                            "type": "number",
                            min: "1",
                            max: "4",
                            value: "{t_depth}",
                            oninput: move |e| t_depth.set(e.value()),
                            "aria-label": "{depth_lbl}",
                        }
                    }
                    label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                        "{kind_lbl}"
                        input {
                            class: "input",
                            placeholder: "works_at · causes:",
                            value: "{t_kind}",
                            oninput: move |e| t_kind.set(e.value()),
                            "aria-label": "{kind_lbl}",
                        }
                    }
                    label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                        "{at_lbl}"
                        input {
                            class: "input",
                            placeholder: "YYYY-MM-DD",
                            value: "{t_at}",
                            oninput: move |e| t_at.set(e.value()),
                            "aria-label": "{at_lbl}",
                        }
                    }
                }
                div { class: "flex items-center gap-3 mt-3",
                    label { class: "flex items-center gap-1.5 text-sm",
                        input {
                            "type": "checkbox",
                            class: "accent-accent",
                            checked: t_cross(),
                            onchange: move |e| t_cross.set(e.value() == "true"),
                        }
                        "{cross_lbl}"
                    }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| {
                            let depth = t_depth().trim().parse().unwrap_or(2).clamp(1, 4);
                            let start = t_start().trim().to_string();
                            if start.is_empty() || !kind_is_valid(&t_kind()) {
                                return;
                            }
                            req.set(Some(TraverseReq {
                                start,
                                depth,
                                kind: t_kind().trim().to_string(),
                                at: t_at().trim().to_string(),
                                cross_domain: t_cross(),
                            }));
                        },
                        "{run_lbl}"
                    }
                }
                { traverse_view(&traverse.read(), &paths_lbl, &rows_lbl) }
            }
        }
    }
}

/// Render the traverse result: the structured `paths` chains (primary) + the
/// flat `traversal` table (back-compat detail). Extracted so the result block
/// is testable-ish and the match is isolated.
fn traverse_view(
    t: &Option<Result<crate::api::TraverseResponse, ApiError>>,
    paths_lbl: &str,
    rows_lbl: &str,
) -> Element {
    match t {
        Some(Ok(r)) if !r.paths.is_empty() => rsx! {
            div { class: "mt-3",
                p { class: "text-sm text-muted-foreground", "{paths_lbl} · {crate::i18n::format_number(r.paths.len() as u64)}" }
                ul { class: "space-y-2 mt-2",
                    for (i, p) in r.paths.iter().enumerate() {
                        li { class: "rounded border border-border p-2 font-mono text-sm",
                            "{i + 1}. {render_path(p)}"
                        }
                    }
                }
                if !r.traversal.is_empty() {
                    details { class: "mt-3",
                        summary { class: "cursor-pointer text-xs text-accent", "{rows_lbl}" }
                        div { class: "overflow-x-auto mt-2",
                            table { class: "table",
                                thead { tr { th { "entity" } th { "depth" } th { "domain" } } }
                                tbody {
                                    for row in &r.traversal {
                                        tr {
                                            td { class: "font-mono text-xs", "{row.entity}" }
                                            td { class: "tabular", "{row.depth}" }
                                            td { class: "text-xs", "{row.domain.clone().unwrap_or_default()}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        Some(Ok(_)) => rsx! { p { class: "text-sm text-muted-foreground mt-3", "no paths" } },
        Some(Err(e)) => {
            rsx! { p { class: "text-danger text-sm mt-3", "{crate::api::error_message(&e)}" } }
        }
        None => rsx! { p { class: "text-muted-foreground text-sm mt-3", "…" } },
    }
}
