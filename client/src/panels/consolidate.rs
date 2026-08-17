//! Create workspace: Consolidate (v1.17.7 M4.3, Write group). Surfaces the
//! v1.6/v1.8 maintenance primitives on a single card:
//! - unresolved contradictions + near-duplicates from `/consolidate/propose`
//! - approve → `/consolidate/apply` (`supersedes` link, expires the prior chunk)
//! - undo → `/consolidate/undo`
//!
//! `propose` is read-only; the operator drives every mutation. No autonomous
//! consolidation (roadmap mandate).

use crate::api::ApiClient;
use dioxus::prelude::*;

#[derive(Debug, Clone)]
enum Item {
    Contradiction { from: i64, to: i64 },
    NearDup { a: i64, b: i64 },
}

pub fn panel() -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<crate::UiState>();
    let writes = (ui.writes_enabled)();

    let mut items = use_signal(Vec::<Item>::new);
    let mut status = use_signal(|| None::<Result<String, String>>);
    let mut loaded = use_signal(|| false);
    let mut loading = use_signal(|| false);

    let mut load = move |_| {
        let api = api;
        loading.set(true);
        spawn(async move {
            let out = match api().consolidate_propose().await {
                Ok(p) => {
                    let mut out = Vec::new();
                    for (from, to) in &p.unresolved_contradictions {
                        out.push(Item::Contradiction {
                            from: *from,
                            to: *to,
                        });
                    }
                    for nd in &p.near_duplicates {
                        let a = nd
                            .get("id_a")
                            .or_else(|| nd.get("a"))
                            .or_else(|| nd.get("from"))
                            .and_then(|v| v.as_i64());
                        let b = nd
                            .get("id_b")
                            .or_else(|| nd.get("b"))
                            .or_else(|| nd.get("to"))
                            .and_then(|v| v.as_i64());
                        if let (Some(a), Some(b)) = (a, b) {
                            out.push(Item::NearDup { a, b });
                        }
                    }
                    out
                }
                Err(e) => {
                    status.set(Some(Err(crate::api::error_message(&e))));
                    Vec::new()
                }
            };
            items.set(out);
            loading.set(false);
            loaded.set(true);
        });
    };

    let run_apply = move |_| {
        let api = api;
        let links: Vec<serde_json::Value> = items()
            .iter()
            .map(|it| match it {
                Item::Contradiction { from, to } => {
                    serde_json::json!({"from_chunk": from, "to_chunk": to, "kind": "supersedes"})
                }
                Item::NearDup { a, b } => {
                    serde_json::json!({"from_chunk": a, "to_chunk": b, "kind": "supersedes"})
                }
            })
            .collect();
        spawn(async move {
            match api()
                .consolidate_apply(&serde_json::Value::Array(links))
                .await
            {
                Ok(r) => {
                    status.set(Some(Ok(crate::i18n::t_fmt(
                        "cons_applied",
                        &[r.recorded.to_string()],
                    ))));
                    load(()); // refresh the propose list
                }
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_undo = move |_| {
        let api = api;
        let ids: Vec<i64> = items()
            .iter()
            .map(|it| match it {
                Item::Contradiction { to, .. } => *to,
                Item::NearDup { b, .. } => *b,
            })
            .collect();
        spawn(async move {
            match api().consolidate_undo(&ids).await {
                Ok(r) => {
                    status.set(Some(Ok(crate::i18n::t_fmt(
                        "cons_undone",
                        &[r.undone.to_string()],
                    ))));
                    load(());
                }
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let cons_title = crate::i18n::t("cons_title");
    let load_lbl = crate::i18n::t("cons_load");
    let apply_lbl = crate::i18n::t("cons_apply");
    let undo_lbl = crate::i18n::t("cons_undo");
    let empty_lbl = crate::i18n::t("cons_empty");
    let near_dup_lbl = crate::i18n::t("cons_near_dup");
    let conflict_lbl = crate::i18n::t("cons_conflict");

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "card-title", "{cons_title}" }
                button { class: "btn btn-outline btn-sm", onclick: move |_| load(()), "{load_lbl}" }
            }
            div { class: "card-body",
                if loading() {
                    p { class: "text-muted-foreground text-sm", "…" }
                } else if items().is_empty() {
                    p { class: "text-muted-foreground text-sm",
                        if loaded() { "{empty_lbl}" } else { "…" }
                    }
                } else {
                    ul { class: "space-y-2",
                        for it in items().iter() {
                            li { class: "rounded border border-border p-2 text-sm",
                                match it {
                                    Item::Contradiction { from, to } => rsx! {
                                        span { class: "badge badge-warn mr-2", "{conflict_lbl}" }
                                        span { class: "font-mono text-xs", "{from} ⟶ {to}" }
                                    },
                                    Item::NearDup { a, b } => rsx! {
                                        span { class: "badge badge-info mr-2", "{near_dup_lbl}" }
                                        span { class: "font-mono text-xs", "{a} ≈ {b}" }
                                    },
                                }
                            }
                        }
                    }
                    div { class: "mt-4 flex items-center gap-3",
                        button { class: "btn btn-primary", disabled: !writes, onclick: run_apply, "{apply_lbl}" }
                        button { class: "btn btn-outline", disabled: !writes, onclick: run_undo, "{undo_lbl}" }
                        div { "role": "status", "aria-live": "polite", class: "text-sm",
                            match status() {
                                Some(Ok(m)) => rsx! { span { class: "text-ok", "{m}" } },
                                Some(Err(m)) => rsx! { span { class: "text-danger", "{m}" } },
                                None => rsx! { span { class: "text-muted-foreground", "…" } },
                            }
                        }
                    }
                }
            }
        }
    }
}
