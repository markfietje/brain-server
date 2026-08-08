//! Recall inspector — the decision-path viewer (DESIGN §4.2).
//! The idiomatic Dioxus pattern: a `use_signal` for the query → a `use_resource`
//! that subscribes to it and auto-refetches on change (cancelling in-flight).
//!
//! v1.16.0 M4: richer hits (per-retriever ranks, fused score, relevance tier,
//! assertion_kind/confidence, decayed/superseded tags), a `min_relevance`
//! slider, and the `?trace=true` decision-path artifact (deep-linkable via
//! `/recall/:trace_id`).

use crate::api::{ApiClient, Hit, RecallResponse};
use crate::Route;
use crate::{DrawerContent, UiState};
use dioxus::prelude::*;

/// Relevance tier → numeric threshold for the `drop_low_relevance` filter.
/// `high` keeps only high; `medium` keeps high+medium; `low`/None keeps all.
fn tier_rank(tier: Option<&str>) -> u8 {
    match tier {
        Some("high") => 3,
        Some("medium") => 2,
        Some("low") => 1,
        _ => 0,
    }
}

/// M4 pure: drop hits whose relevance tier is below the minimum. The client-
/// side post-fusion filter (the backend applies the same on request; here it's
/// a live visual control, so client-side). Hits without a relevance tag are
/// kept (the backend tags relevance only when it computed one).
pub fn drop_low_relevance(hits: Vec<Hit>, min: Option<&str>) -> Vec<Hit> {
    let floor = tier_rank(min);
    hits.into_iter()
        .filter(|h| h.relevance.is_none() || tier_rank(h.relevance.as_deref()) >= floor)
        .collect()
}

pub fn panel() -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let writes = (ui.writes_enabled)();
    let mut query = use_signal(String::new);
    let mut trace = use_signal(|| false); // M4.2: ?trace=true toggle
    let mut min_rel = use_signal(String::new); // M4.1: high|medium|low|""

    // use_resource subscribes to `query` + `trace` + `min_rel` → reruns on change.
    let recall = use_resource(move || {
        let q = query();
        let trace = trace();
        let min = min_rel();
        let api = api();
        async move {
            api.recall(&q, trace, if min.is_empty() { None } else { Some(&min) })
                .await
        }
    });

    rsx! {
        h1 { "Recall inspector" }
        input {
            class: "border border-border-subtle surface-raised rounded px-2 py-1 w-full",
            placeholder: "query brain-server (min 5 chars)…",
            value: "{query}",
            oninput: move |e| query.set(e.value()),
            "aria-label": "recall query",
        }
        div { class: "flex gap-3 my-2 items-center text-sm",
            // M4.2: the trace toggle produces a deep-linkable trace_id.
            label { class: "flex items-center gap-1",
                input {
                    "type": "checkbox",
                    checked: trace(),
                    onchange: move |e| trace.set(e.value() == "true"),
                    disabled: !writes,
                }
                "trace decision path"
            }
            // M4.1: min_relevance post-fusion filter (the "stop poisoning the
            // context window" slider — deterministic, zero-token).
            label { class: "flex items-center gap-1",
                "min relevance"
                select {
                    class: "border border-border-subtle surface-raised rounded px-1 py-0.5",
                    value: "{min_rel}",
                    onchange: move |e| min_rel.set(e.value()),
                    option { value: "", "any" }
                    option { value: "medium", "medium+" }
                    option { value: "high", "high" }
                }
            }
        }
        { recall_view(&recall.read(), &min_rel(), trace) }
    }
}

/// Render the recall result block. Extracted so `trace_panel` reuses the
/// per-hit row rendering without duplicating `HitRow`.
fn recall_view(
    recall: &Option<Result<RecallResponse, crate::api::ApiError>>,
    min_rel: &str,
    _trace: Signal<bool>,
) -> Element {
    match recall {
        Some(Ok(r)) if !r.hits.is_empty() => {
            let hits = drop_low_relevance(
                r.hits.clone(),
                if min_rel.is_empty() {
                    None
                } else {
                    Some(min_rel)
                },
            );
            rsx! {
                div {
                    p { class: "text-sm text-ink-muted", "decision: {r.decision} · {hits.len()} hits" }
                    ul { class: "mt-2 divide-y hairline",
                        for h in &hits { HitRow { hit: h.clone() } }
                    }
                    // M4.2: the trace artifact is deep-linkable — render a link
                    // to /recall/:trace_id so the decision path is shareable.
                    if let Some(tid) = r.trace_id {
                        p { class: "mt-2 text-xs text-ink-muted",
                            Link { to: Route::RecallTrace { trace_id: tid },
                                "decision-path trace #{tid} ↗"
                            }
                        }
                    }
                }
            }
        }
        Some(Ok(_)) => rsx! { p { class: "text-ink-muted mt-2", "no hits" } },
        Some(Err(e)) => rsx! { p { class: "text-danger mt-2", "recall failed: {e}" } },
        None => rsx! { p { class: "text-ink-muted mt-2", "…" } },
    }
}

#[component]
fn HitRow(hit: Hit) -> Element {
    let mut ui = use_context::<UiState>();
    let prov = hit.provenance.clone();
    let rel = hit.relevance.clone();
    rsx! {
        li { class: "py-2",
            div { class: "flex justify-between items-center",
                button {
                    class: "font-mono text-sm text-accent hover:underline text-left",
                    onclick: move |_| ui.drawer.set(Some(DrawerContent::Hit(hit.clone()))),
                    "chunk #{hit.id}"
                }
                // M4.1: per-retriever ranks + fused score (monospace, tabular).
                span { class: "font-mono text-xs text-ink-muted tabular",
                    {
                        let v = prov.as_ref().and_then(|p| p.vector_rank).map(|_| "v").unwrap_or("");
                        let f = prov.as_ref().and_then(|p| p.fts_rank).map(|_| "f").unwrap_or("");
                        let g = prov.as_ref().and_then(|p| p.graph_rank).map(|_| "g").unwrap_or("");
                        let fused = prov.as_ref().and_then(|p| p.fused_score).map(|s| format!(" {s:.2}")).unwrap_or_default();
                        format!("{v}{f}{g} score {score:.3}{fused}", score = hit.score)
                    }
                }
            }
            div { class: "flex gap-2 text-xs mt-0.5 flex-wrap",
                if let Some(src) = &hit.source {
                    span { class: "text-ink-faint", "via {src}" }
                }
                if let Some(r) = &rel {
                    span { class: relevance_color(r), "relevance: {r}" }
                }
                if let Some(a) = &hit.assertion_kind {
                    span { class: "text-info", "{a}" }
                }
                if let Some(c) = hit.confidence {
                    span { class: "text-ink-muted tabular", "conf {c:.2}" }
                }
                if hit.decayed == Some(true) {
                    span { class: "text-warn", "decayed" }
                }
                if hit.conflict == Some(true) {
                    span { class: "text-warn", "superseded" }
                }
            }
            p { class: "text-sm text-ink mt-1", "{hit.content}" }
        }
    }
}

fn relevance_color(tier: &str) -> &'static str {
    match tier {
        "high" => "text-ok",
        "medium" => "text-info",
        "low" => "text-ink-faint",
        _ => "",
    }
}

/// M4.2: stringify a JSON value's field for the trace table. Numbers, strings,
/// and bools render as their natural form; missing → "—". ponytail: a tiny
/// helper avoids `Option<&Value>` Display (which doesn't exist).
fn json_str(v: &serde_json::Value, key: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        Some(other) => other.to_string(),
        None => "—".into(),
    }
}

/// M4.2: one trace hit row. Extracted so the rsx string-interpolation of the
/// `json_str` calls doesn't trip the macro's nested-quote parsing.
fn trace_hit_row(h: &serde_json::Value) -> Element {
    let id = json_str(h, "id");
    let score = json_str(h, "score");
    let source = json_str(h, "source");
    let relevance = json_str(h, "relevance");
    rsx! {
        tr {
            td { class: "pr-2 font-mono", "{id}" }
            td { class: "pr-2 font-mono", "{score}" }
            td { class: "pr-2", "{source}" }
            td { class: "pr-2", "{relevance}" }
        }
    }
}

/// M4.2: the deep-linkable trace artifact. Fetches `GET /recall/:trace_id/trace`
/// and renders the replayable decision path: query, decision, domains searched,
/// applied scope, actor, per-hit id/score/source/relevance. This is the Intent-
/// Based-Auditing decision-path pillar + the Art 22 "meaningful information
/// about the logic" evidence.
pub fn trace_panel(trace_id: i64) -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let trace = use_resource(move || {
        let api = api();
        async move { api.recall_trace(trace_id).await }
    });
    rsx! {
        h1 { "Recall trace #{trace_id}" }
        p { class: "text-xs text-ink-muted mb-2",
            "the recorded decision path for a past recall (replayable audit artifact)" }
        match &*trace.read() {
            Some(Ok(v)) => rsx! { TraceCard { trace: v.clone() } },
            Some(Err(e)) => rsx! { p { class: "text-danger mt-2", "trace failed: {e}" } },
            None => rsx! { p { class: "text-ink-muted mt-2", "loading…" } },
        }
        p { class: "mt-3" , Link { to: Route::Recall {}, "← back to recall" } }
    }
}

/// Render the trace JSON. The server stores the full decision-path metadata
/// (query, decision, domains, scope, actor, per-hit id/score/source/relevance);
/// we render each known key, tolerating additions.
#[component]
fn TraceCard(trace: serde_json::Value) -> Element {
    let q = trace.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let decision = trace.get("decision").and_then(|v| v.as_str()).unwrap_or("");
    let actor = trace.get("actor").and_then(|v| v.as_str()).unwrap_or("");
    let domains = trace.get("domains_searched").and_then(|v| v.as_array());
    let scope = trace.get("applied_scope").and_then(|v| v.as_str());
    let hits = trace.get("hits").and_then(|v| v.as_array());
    rsx! {
        div { class: "surface-raised border hairline rounded p-3 text-sm",
            dl { class: "grid grid-cols-[auto_1fr] gap-x-3 gap-y-1",
                dt { class: "text-ink-muted", "query" }    dd { "{q}" }
                dt { class: "text-ink-muted", "decision" } dd { class: "font-mono", "{decision}" }
                dt { class: "text-ink-muted", "actor" }    dd { class: "font-mono", "{actor}" }
                if let Some(s) = scope { dt { class: "text-ink-muted", "scope" } dd { "{s}" } }
            }
            if let Some(domains) = domains {
                p { class: "mt-2 text-xs text-ink-muted",
                    "domains: "
                    { domains.iter().filter_map(|d| d.as_str()).collect::<Vec<_>>().join(", ") }
                }
            }
            if let Some(hits) = hits {
                table { class: "w-full text-xs tabular mt-2",
                    thead { tr {
                        th { class: "text-left pr-2", "id" }
                        th { class: "text-left pr-2", "score" }
                        th { class: "text-left pr-2", "source" }
                        th { class: "text-left pr-2", "relevance" }
                    } }
                    tbody {
                        for h in hits {
                            { trace_hit_row(h) }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// M4 tests — wire pin for richer hits + the pure relevance filter.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: i64, relevance: Option<&str>) -> Hit {
        Hit {
            id,
            title: None,
            content: format!("c{id}"),
            snippet: None,
            score: 0.5,
            domain: None,
            source: None,
            conflict: None,
            provenance: None,
            assertion_kind: None,
            confidence: None,
            relevance: relevance.map(str::to_string),
            decayed: None,
        }
    }

    /// `drop_low_relevance` filters hits below the tier threshold; hits with no
    /// tier tag are kept (the backend tags relevance only when it computed one).
    #[test]
    fn drop_low_relevance_filters_below_tier() {
        let hits = vec![
            hit(1, Some("high")),
            hit(2, Some("medium")),
            hit(3, Some("low")),
            hit(4, None),
        ];
        let medium = drop_low_relevance(hits.clone(), Some("medium"));
        assert_eq!(
            medium.iter().map(|h| h.id).collect::<Vec<_>>(),
            vec![1, 2, 4]
        );
        let high = drop_low_relevance(hits, Some("high"));
        assert_eq!(high.iter().map(|h| h.id).collect::<Vec<_>>(), vec![1, 4]);
    }

    /// M5.1/M4 shared: the relevance tier color maps to state tokens.
    #[test]
    fn relevance_tier_color_maps_to_state_tokens() {
        assert_eq!(relevance_color("high"), "text-ok");
        assert_eq!(relevance_color("medium"), "text-info");
        assert_eq!(relevance_color("low"), "text-ink-faint");
        assert_eq!(relevance_color("unknown"), "");
    }
}
