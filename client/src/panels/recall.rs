//! Recall inspector — the decision-path viewer (DESIGN §4.2).
//! The idiomatic Dioxus pattern: a `use_signal` for the query → a `use_resource`
//! that subscribes to it and auto-refetches on change (cancelling in-flight).
//!
//! v1.16.0 M4: richer hits (per-retriever ranks, fused score, relevance tier,
//! assertion_kind/confidence, decayed/superseded tags), a `min_relevance`
//! slider, and the `?trace=true` decision-path artifact (deep-linkable via
//! `/recall/:trace_id`).

use crate::api::{error_message, ApiClient, Hit, RecallResponse};
use crate::panels::{use_document_title, PageTitle};
use crate::Route;
use crate::{DrawerContent, UiState};
use dioxus::prelude::*;

/// v1.16.7 M6: search-as-you-type debounce. Wait this long after the last
/// keystroke before firing `/recall`, so a fast typist sends one request per
/// pause instead of one per character. 300ms is the 2026 industry default.
const DEBOUNCE_MS: u32 = 300;

/// v1.16.7 M6 pure: the debounce cancel-safety rule. `gen_at_spawn` is the
/// keystroke generation when the delayed commit was scheduled; `gen_now` is the
/// current generation when the delay elapses. Commit only if they match — a
/// newer keystroke bumped the generation, so the scheduled value is stale and
/// must be dropped (the newer keystroke scheduled its own commit). This is the
/// cancel-safety check that replaces real future-cancellation (Dioxus does not
/// cancel `spawn`ed futures when an effect re-runs).
fn debounce_commit(gen_at_spawn: u64, gen_now: u64) -> bool {
    gen_at_spawn == gen_now
}

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
    use_document_title(|| "Recall — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    // v1.16.7 M6: two signals — `input` (raw, per keystroke, drives the box) and
    // `query` (debounced, drives the use_resource). The resource never sees a
    // per-keystroke value, so it only refetches once per pause.
    let mut input = use_signal(String::new);
    let mut query = use_signal(String::new);
    // The keystroke generation: bumped on every input; a delayed commit commits
    // only if its generation still matches (cancel-safe — see `debounce_commit`).
    let mut gen = use_signal(|| 0u64);
    let mut trace = use_signal(|| false); // M4.2: ?trace=true toggle
    let mut min_rel = use_signal(String::new); // M4.1: high|medium|low|""

    // v1.16.7 M6: schedule a debounced commit on every keystroke. The delayed
    // future sleeps via the JS engine (dependency-free, mirrors probe_sleep in
    // main.rs), then checks the generation — a newer keystroke cancels it.
    let oninput = move |e: Event<FormData>| {
        input.set(e.value());
        gen += 1;
        let gen_at_spawn = gen();
        let val = input();
        spawn(async move {
            // Dependency-free sleep: web + desktop webviews both have a JS engine.
            let _ = document::eval(&format!(
                "return await new Promise(r => setTimeout(r, {DEBOUNCE_MS}));"
            ))
            .await;
            if debounce_commit(gen_at_spawn, gen()) {
                query.set(val);
            }
        });
    };

    // use_resource subscribes to `query` + `trace` + `min_rel` → reruns on change.
    let recall = use_resource(move || {
        let q = query();
        let trace = trace();
        let min = min_rel();
        let api = api();
        async move {
            api.recall(
                &q,
                trace,
                if min.is_empty() { None } else { Some(&min) },
                false,
            )
            .await
        }
    });

    rsx! {
        PageTitle { {crate::i18n::t("recall_title")} }
        input {
            class: "input w-full",
            placeholder: "query brain-server (min 5 chars)…",
            value: "{input}",
            oninput,
            "aria-label": "recall query",
        }
        div { class: "flex gap-4 my-3 items-center text-sm flex-wrap",
            // M4.2: the trace toggle produces a deep-linkable trace_id.
            label { class: "flex items-center gap-1.5",
                input {
                    "type": "checkbox",
                    class: "accent-accent",
                    checked: trace(),
                    onchange: move |e| trace.set(e.value() == "true"),
                    // M1 amber rule: trace is a READ control (re-recall only);
                    // reads stay interactive during Reconnecting — writes freeze,
                    // reads don't (DESIGN §6). Matches the query input + select.
                }
                "trace decision path"
            }
            // M4.1: min_relevance post-fusion filter (the "stop poisoning the
            // context window" slider — deterministic, zero-token).
            label { class: "flex items-center gap-1.5",
                "min relevance"
                select {
                    class: "select",
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
                div { class: "mt-2 space-y-3",
                    p { class: "text-sm text-muted-foreground", "decision: {r.decision} · {hits.len()} hits" }
                    ul { class: "divide-y divide-border",
                        for h in &hits { HitRow { hit: h.clone() } }
                    }
                    // M4.2: the trace artifact is deep-linkable — render a link
                    // to /recall/:trace_id so the decision path is shareable.
                    if let Some(tid) = r.trace_id {
                        p { class: "text-xs text-muted-foreground",
                            Link { to: Route::RecallTrace { trace_id: tid },
                                "decision-path trace #{tid} ↗"
                            }
                        }
                    }
                }
            }
        }
        Some(Ok(_)) => rsx! { p { class: "text-muted-foreground mt-2", "no hits" } },
        Some(Err(e)) => {
            rsx! { p { class: "text-danger mt-2", "recall failed: {error_message(&e)}" } }
        }
        None => rsx! { p { class: "text-muted-foreground mt-2", "…" } },
    }
}

#[component]
fn HitRow(hit: Hit) -> Element {
    let mut ui = use_context::<UiState>();
    let prov = hit.provenance.clone();
    let rel = hit.relevance.clone();
    rsx! {
        li { class: "py-3",
            div { class: "flex justify-between items-center gap-2",
                button {
                    class: "font-mono text-sm text-accent hover:underline text-left",
                    onclick: move |_| ui.drawer.set(Some(DrawerContent::Hit(hit.clone()))),
                    "chunk #{hit.id}"
                }
                // M4.1: per-retriever ranks + fused score (monospace, tabular).
                span { class: "font-mono text-xs text-muted-foreground tabular",
                    {
                        let v = prov.as_ref().and_then(|p| p.vector_rank).map(|_| "v").unwrap_or("");
                        let f = prov.as_ref().and_then(|p| p.fts_rank).map(|_| "f").unwrap_or("");
                        let g = prov.as_ref().and_then(|p| p.graph_rank).map(|_| "g").unwrap_or("");
                        let fused = prov.as_ref().and_then(|p| p.fused_score).map(|s| format!(" {s:.2}")).unwrap_or_default();
                        format!("{v}{f}{g} score {score:.3}{fused}", score = hit.score)
                    }
                }
            }
            div { class: "flex gap-2 text-xs mt-0.5 flex-wrap items-center",
                if let Some(src) = &hit.source {
                    span { class: "badge", "via {src}" }
                }
                if let Some(r) = &rel {
                    span { class: relevance_color(r), "relevance: {r}" }
                }
                if let Some(a) = &hit.assertion_kind {
                    span { class: "text-info", "{a}" }
                }
                if let Some(c) = hit.confidence {
                    span { class: "text-muted-foreground tabular", "conf {c:.2}" }
                }
                if hit.decayed == Some(true) {
                    span { class: "text-warn", "decayed" }
                }
                if hit.conflict == Some(true) {
                    span { class: "text-warn", "superseded" }
                }
            }
            p { class: "text-sm text-foreground mt-1", "{crate::strip_invisible(&hit.content)}" }
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

/// v1.20.20 M1: a scalar replay field, run through the v1.20.3 `strip_invisible`
/// render boundary (traces can carry smuggling bytes in stored metadata — the
/// same class the operator surfaces close). Missing → "—". Pure + used by the
/// renderer so the replay view de-obfuscates exactly like every other surface.
fn replay_str(v: &serde_json::Value, key: &str) -> String {
    crate::strip_invisible(&json_str(v, key))
}

/// v1.20.20 M1: a replay list field (`domains_searched` / applied `scope`,
/// both stored as JSON arrays) joined into a stripped display string. Missing
/// or empty → "—". Pure so the header card is testable.
fn replay_list(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|val| val.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.as_str())
                .map(crate::strip_invisible)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".into())
}

fn trace_hit_row(h: &serde_json::Value) -> Element {
    let id = replay_str(h, "id");
    let score = replay_str(h, "score");
    let source = replay_str(h, "source");
    let relevance = replay_str(h, "relevance");
    let assertion = replay_str(h, "assertion_kind");
    let decayed = trace_decayed_marker(h);
    rsx! {
        tr {
            td { class: "pr-2 font-mono", "{id}" }
            td { class: "pr-2 font-mono", "{score}" }
            td { class: "pr-2", "{source}" }
            td { class: "pr-2", "{relevance}" }
            td { class: "pr-2 text-info", "{assertion}" }
            td { class: "pr-2", "{decayed}" }
        }
    }
}

/// The per-hit `decayed` evidence marker for the trace table: `true` → the
/// chunk had expired at recall time (evidence-quality signal); anything else
/// (absent/false) → "—". Pure so the trace rows are testable.
fn trace_decayed_marker(h: &serde_json::Value) -> &'static str {
    if json_str(h, "decayed") == "true" {
        "decayed"
    } else {
        "—"
    }
}

/// M4.2: the deep-linkable trace artifact. Fetches `GET /recall/:trace_id/trace`
/// and renders the replayable decision path: query, decision, domains searched,
/// applied scope, actor, per-hit id/score/source/relevance. This is the Intent-
/// Based-Auditing decision-path pillar + the Art 22 "meaningful information
/// about the logic" evidence.
pub fn trace_panel(trace_id: i64) -> Element {
    let title = crate::i18n::t("replay_title");
    let export_label = crate::i18n::t("replay_export");
    let doc_title = title.clone();
    use_document_title(move || format!("{doc_title} #{trace_id} — brain"));
    let api = use_context::<Signal<ApiClient>>();
    let trace = use_resource(move || {
        let api = api();
        async move { api.recall_trace(trace_id).await }
    });
    // v1.20.20 M3: the raw fetched trace JSON — held here so the export button
    // downloads the exact evidence without a second fetch. The download JS is
    // built once (owned String) so the 'static onclick closure can move it.
    let export_el = match &*trace.read() {
        Some(Ok(v)) => {
            let body = v.to_string();
            rsx! {
                div { class: "mt-3 flex gap-2 items-center",
                    button {
                        class: "btn btn-outline btn-md",
                        onclick: move |_| {
                            let js = format!(
                                "(function(){{var b=new Blob([{body:?}],{{type:'application/json'}});var u=URL.createObjectURL(b);var a=document.createElement('a');a.href=u;a.download='trace-{trace_id}.json';a.click();URL.revokeObjectURL(u);}})();"
                            );
                            let _ = document::eval(&js);
                        },
                        "{export_label}"
                    }
                    Link { to: Route::Recall {}, "← back to recall" }
                }
            }
        }
        _ => rsx! { p { class: "mt-3" , Link { to: Route::Recall {}, "← back to recall" } } },
    };
    rsx! {
        PageTitle { "{title} #{trace_id}" }
        p { class: "text-xs text-muted-foreground mb-2",
            "the recorded decision path for a past recall (replayable audit artifact)" }
        match &*trace.read() {
            Some(Ok(v)) => rsx! { TraceCard { trace: v.clone() } },
            Some(Err(e)) => rsx! { p { class: "text-danger mt-2", "trace failed: {error_message(&e)}" } },
            None => rsx! { p { class: "text-muted-foreground mt-2", "loading…" } },
        }
        { export_el }
    }
}

/// Render the trace JSON. The server stores the full decision-path metadata
/// (`query_hash` after v1.20.17 M3, decision, actor, domains_searched, applied
/// scope, per-hit id/score/source/relevance); we render each known key,
/// tolerating additions. Every displayed string crosses the v1.20.3
/// `strip_invisible` boundary (`replay_str`/`replay_list`) — a trace can carry
/// original hit metadata that was never screened.
#[component]
fn TraceCard(trace: serde_json::Value) -> Element {
    let q = replay_str(&trace, "query_hash");
    let decision = replay_str(&trace, "decision");
    let actor = replay_str(&trace, "actor");
    let scope = replay_list(&trace, "scope");
    let domains = replay_list(&trace, "domains_searched");
    let hits = trace.get("hits").and_then(|v| v.as_array());
    rsx! {
        div { class: "card p-3 text-sm",
            dl { class: "grid grid-cols-[auto_1fr] gap-x-3 gap-y-1",
                dt { class: "text-muted-foreground", "query_hash" } dd { "{q}" }
                dt { class: "text-muted-foreground", "decision" } dd { class: "font-mono", "{decision}" }
                dt { class: "text-muted-foreground", "actor" }    dd { class: "font-mono", "{actor}" }
                dt { class: "text-muted-foreground", "scope" }    dd { "{scope}" }
            }
            p { class: "mt-2 text-xs text-muted-foreground", "domains: {domains}" }
            if let Some(hits) = hits {
                table { class: "table mt-2",
                    thead { tr {
                        th { class: "text-left pr-2", "id" }
                        th { class: "text-left pr-2", "score" }
                        th { class: "text-left pr-2", "source" }
                        th { class: "text-left pr-2", "relevance" }
                        th { class: "text-left pr-2", "kind" }
                        th { class: "text-left pr-2", "decayed" }
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
            flagged: None,
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

    /// v1.16.7 M6: the debounce cancel-safety rule. A delayed commit fires only
    /// if no newer keystroke bumped the generation in the meantime.
    #[test]
    fn debounce_commits_only_when_generation_unchanged() {
        // Generation matched at spawn and now → commit.
        assert!(debounce_commit(3, 3));
        // A newer keystroke bumped the generation → the scheduled value is stale.
        assert!(!debounce_commit(3, 4));
        // An older/lower generation can't be current → never commits.
        assert!(!debounce_commit(5, 3));
    }

    /// The trace table's per-hit evidence marker: `decayed:true` at recall
    /// time → flagged; absent/false → "—".
    #[test]
    fn trace_decayed_marker_flags_only_true() {
        let d = |decayed: Option<bool>| {
            let mut m = serde_json::Map::new();
            if let Some(b) = decayed {
                m.insert("decayed".into(), serde_json::json!(b));
            }
            trace_decayed_marker(&serde_json::Value::Object(m))
        };
        assert_eq!(d(Some(true)), "decayed");
        assert_eq!(d(Some(false)), "—");
        assert_eq!(d(None), "—");
    }

    /// v1.20.20 M1: the replay header reads the *stored* shape — `query_hash`
    /// (not `query`, v1.20.17 M3) and the applied `scope` array — and runs every
    /// displayed string through the `strip_invisible` render boundary. A trace
    /// can carry original hit/scope metadata that was never screened.
    #[test]
    fn replay_header_reads_stored_shape_and_strips() {
        let trace = serde_json::json!({
            "query_hash": "abc123",
            "decision": "include\u{202E}gnahc",
            "actor": "cli",
            "domains_searched": ["global", "fin\u{200B}ance"],
            "scope": ["alice", "e\u{202E}vil"],
            "hits": []
        });
        // query_hash, not query — a stale `query` key must not leak through.
        assert_eq!(replay_str(&trace, "query_hash"), "abc123");
        assert_eq!(replay_str(&trace, "query"), "—");
        // Stripped: no bidi override (U+202E) / zero-width (U+200B) survive.
        assert!(!replay_str(&trace, "decision").contains('\u{202E}'));
        assert!(!replay_list(&trace, "domains_searched").contains('\u{200B}'));
        assert!(!replay_list(&trace, "scope").contains('\u{202E}'));
        // A missing list field renders "—", never a panic.
        assert_eq!(replay_list(&trace, "absent"), "—");
    }

    /// v1.20.20 M1: the per-hit replay cells cross the same render boundary — a
    /// U+202E smuggled into a hit's `source` must be stripped before display.
    #[test]
    fn replay_hit_cells_strip_smuggled_bidi() {
        let hit = serde_json::json!({
            "id": 7,
            "score": 0.42,
            "source": "mark\u{202E}dlo",
            "relevance": "high",
            "assertion_kind": "fact",
            "decayed": false
        });
        assert!(!replay_str(&hit, "source").contains('\u{202E}'));
        assert_eq!(replay_str(&hit, "score"), "0.42");
        assert_eq!(replay_str(&hit, "missing"), "—");
    }
}
