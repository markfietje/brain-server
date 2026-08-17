//! Create workspace: Procedures (v1.17.7 M4.2, Write group). Two cards —
//! a step builder that POSTs `/procedure` and lists steps via `/procedure/{id}/steps`,
//! plus the two deterministic helpers that make a procedure useful in practice:
//! `/classify` (keyword router) and `/decision/{id}/evaluate` (variable-bound
//! decision rules). No LLM anywhere — these are the deterministic v1.10 primitives.

use crate::api::{ApiClient, StepView};
use dioxus::prelude::*;

pub fn panel() -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<crate::UiState>();
    let writes = (ui.writes_enabled)();

    // Procedure builder.
    let mut p_title = use_signal(String::new);
    let mut p_content = use_signal(String::new);
    let mut p_domain = use_signal(String::new);
    let mut steps = use_signal(Vec::<(String, String, bool)>::new);
    let mut step_title = use_signal(String::new);
    let mut step_content = use_signal(String::new);
    let mut step_decision = use_signal(|| false);
    // Classifier.
    let mut cls_text = use_signal(String::new);
    // Decision evaluate.
    let mut dec_id = use_signal(String::new);
    let mut dec_vars = use_signal(String::new);

    let mut status = use_signal(|| None::<Result<String, String>>);
    let mut steps_view = use_signal(|| None::<Vec<StepView>>);
    let mut cls_result = use_signal(String::new);
    let mut dec_result = use_signal(String::new);

    let run_create = move |_| {
        let api = api;
        let t = p_title();
        let c = p_content();
        let d = p_domain();
        let st: Vec<serde_json::Value> = steps()
            .iter()
            .filter(|(t, _, _)| !t.trim().is_empty())
            .map(|(t, c, dec)| serde_json::json!({ "title": t, "content": c, "is_decision": dec }))
            .collect();
        spawn(async move {
            match api()
                .procedure_create(&t, &c, &serde_json::Value::Array(st), &d)
                .await
            {
                Ok(pr) => {
                    let n = pr.step_ids.len();
                    let id = pr.id;
                    status.set(Some(Ok(crate::i18n::t_fmt(
                        "proc_created",
                        &[n.to_string()],
                    ))));
                    // Fetch the ordered steps for the new procedure, best-effort.
                    steps_view.set(api().procedure_steps(id).await.ok().map(|r| r.steps));
                }
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_classify = move |_| {
        let api = api;
        let t = cls_text();
        spawn(async move {
            match api().classify(&t).await {
                Ok(cr) => {
                    let kws = cr.result.matched_keywords.join(", ");
                    let s = if kws.is_empty() {
                        format!("{} ({:.2})", cr.result.category, cr.result.confidence)
                    } else {
                        format!(
                            "{} ({:.2}) → [{kws}]",
                            cr.result.category, cr.result.confidence
                        )
                    };
                    cls_result.set(s);
                }
                Err(e) => cls_result.set(crate::api::error_message(&e)),
            }
        });
    };

    let run_decision = move |_| {
        let api = api;
        let id = match dec_id().trim().parse::<i64>() {
            Ok(id) => id,
            Err(_) => {
                dec_result.set("invalid decision id".into());
                return;
            }
        };
        let vars = dec_vars();
        let map = parse_decision_vars(&vars);
        let val = serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                .collect(),
        );
        spawn(async move {
            match api().decision_evaluate(id, &val).await {
                Ok(d) => {
                    let mut s = d.result;
                    if let Some(cond) = &d.matched_condition {
                        s.push_str(&format!(" (matched: {cond})"));
                    }
                    if let Some(cit) = &d.citation {
                        s.push_str(&format!(" · {cit}"));
                    }
                    if d.used_default {
                        s.push_str(" · default");
                    }
                    dec_result.set(s);
                }
                Err(e) => dec_result.set(crate::api::error_message(&e)),
            }
        });
    };

    let add_step = move |_| {
        let t = step_title();
        let c = step_content();
        let d = step_decision();
        if t.trim().is_empty() {
            return;
        }
        steps.with_mut(|v| v.push((t, c, d)));
        step_title.set(String::new());
        step_content.set(String::new());
        step_decision.set(false);
    };

    let proc_title = crate::i18n::t("proc_title");
    let step_title_lbl = crate::i18n::t("proc_step_title");
    let step_body_lbl = crate::i18n::t("proc_step_body");
    let add_lbl = crate::i18n::t("proc_add_step");
    let create_lbl = crate::i18n::t("proc_create");
    let steps_lbl = crate::i18n::t("proc_steps");
    let is_decision_lbl = crate::i18n::t("proc_is_decision");
    let cls_title = crate::i18n::t("cls_title");
    let cls_text_lbl = crate::i18n::t("cls_text");
    let cls_run_lbl = crate::i18n::t("cls_run");
    let dec_title = crate::i18n::t("dec_title");
    let dec_id_lbl = crate::i18n::t("dec_id");
    let dec_vars_lbl = crate::i18n::t("dec_vars");
    let dec_run_lbl = crate::i18n::t("dec_run");
    let content_lbl = crate::i18n::t("ingest_content");
    let domain_lbl = crate::i18n::t("ingest_domain");
    let ingest_title = crate::i18n::t("ingest_title");

    rsx! {
        div { class: "card",
            div { class: "card-header", div { class: "card-title", "{proc_title}" } }
            div { class: "card-body",
                div { class: "grid gap-3 md:grid-cols-2",
                    label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                        "{ingest_title}"
                        input { class: "input", value: "{p_title}", oninput: move |e| p_title.set(e.value()), "aria-label": "{ingest_title}" }
                    }
                    label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                        "{domain_lbl}"
                        input { class: "input", placeholder: "global", value: "{p_domain}", oninput: move |e| p_domain.set(e.value()), "aria-label": "{domain_lbl}" }
                    }
                }
                label { class: "flex flex-col gap-1 text-xs text-muted-foreground mt-3",
                    "{content_lbl}"
                    textarea { class: "input min-h-24", value: "{p_content}", oninput: move |e| p_content.set(e.value()), "aria-label": "{content_lbl}" }
                }
                // Step builder.
                div { class: "mt-4 rounded border border-border p-3",
                    p { class: "text-sm font-semibold mb-2", "{steps_lbl}" }
                    div { class: "grid gap-2 md:grid-cols-2",
                        input { class: "input", placeholder: "{step_title_lbl}", value: "{step_title}", oninput: move |e| step_title.set(e.value()) }
                        textarea { class: "input min-h-16", placeholder: "{step_body_lbl}", value: "{step_content}", oninput: move |e| step_content.set(e.value()) }
                    }
                    div { class: "flex items-center gap-3 mt-2",
                        label { class: "flex items-center gap-1.5 text-sm",
                            input { "type": "checkbox", class: "accent-accent", checked: step_decision(), onchange: move |e| step_decision.set(e.value() == "true") }
                            "{is_decision_lbl}"
                        }
                        button { class: "btn btn-outline btn-sm", onclick: add_step, "{add_lbl}" }
                    }
                    if !steps().is_empty() {
                        ul { class: "mt-3 space-y-1",
                            for (i, (t, _, d)) in steps().iter().enumerate() {
                                li { class: "text-sm flex items-center gap-2",
                                    span { class: "font-mono text-xs", "{i}. " }
                                    span { "{t}" }
                                    if *d { span { class: "badge badge-info", "{is_decision_lbl}" } }
                                }
                            }
                        }
                    }
                }
                div { class: "mt-4 flex items-center gap-3",
                    button { class: "btn btn-primary", disabled: !writes, onclick: run_create, "{create_lbl}" }
                    div { "role": "status", "aria-live": "polite", class: "text-sm",
                        match status() {
                            Some(Ok(m)) => rsx! { span { class: "text-ok", "{m}" } },
                            Some(Err(m)) => rsx! { span { class: "text-danger", "{m}" } },
                            None => rsx! { span { class: "text-muted-foreground", "…" } },
                        }
                    }
                }
                if let Some(sv) = steps_view() {
                    div { class: "mt-4",
                        p { class: "text-sm font-semibold mb-2", "{steps_lbl}" }
                        ol { class: "space-y-2",
                            for s in sv {
                                li { class: "rounded border border-border p-2",
                                    div { class: "flex items-center gap-2",
                                        span { class: "font-mono text-xs", "{s.step_index}" }
                                        span { class: "font-semibold text-sm", "{s.title.clone().unwrap_or_default()}" }
                                        span { class: "badge badge-neutral text-xs", "{s.memory_kind}" }
                                    }
                                    if !s.content.is_empty() { p { class: "text-sm text-muted-foreground mt-1", "{crate::strip_invisible(&s.content)}" } }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Classify card.
        div { class: "card mt-4",
            div { class: "card-header", div { class: "card-title", "{cls_title}" } }
            div { class: "card-body",
                label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                    "{cls_text_lbl}"
                    input { class: "input", value: "{cls_text}", oninput: move |e| cls_text.set(e.value()), "aria-label": "{cls_text_lbl}" }
                }
                div { class: "mt-3 flex items-center gap-3",
                    button { class: "btn btn-secondary", onclick: run_classify, "{cls_run_lbl}" }
                    pre { class: "text-xs whitespace-pre-wrap", "{cls_result}" }
                }
            }
        }
        // Decision evaluate card.
        div { class: "card mt-4",
            div { class: "card-header", div { class: "card-title", "{dec_title}" } }
            div { class: "card-body",
                div { class: "grid gap-3 md:grid-cols-2",
                    label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                        "{dec_id_lbl}"
                        input { class: "input", value: "{dec_id}", oninput: move |e| dec_id.set(e.value()), "aria-label": "{dec_id_lbl}" }
                    }
                    label { class: "flex flex-col gap-1 text-xs text-muted-foreground",
                        "{dec_vars_lbl}"
                        input { class: "input font-mono", placeholder: "revenue: 1200", value: "{dec_vars}", oninput: move |e| dec_vars.set(e.value()), "aria-label": "{dec_vars_lbl}" }
                    }
                }
                div { class: "mt-3 flex items-center gap-3",
                    button { class: "btn btn-secondary", onclick: run_decision, "{dec_run_lbl}" }
                    pre { class: "text-xs whitespace-pre-wrap", "{dec_result}" }
                }
            }
        }
    }
}

/// v1.17.7 M4.2: turn a malformed decision-variables string into a lenient map.
/// `Ok` when the text is empty or valid JSON with numeric leaves; the map is
/// best-effort (non-numeric entries are dropped, not fatal). Pure + testable.
pub fn parse_decision_vars(s: &str) -> std::collections::HashMap<String, f64> {
    serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f)))
                .collect()
        })
        .unwrap_or_default()
}
