//! Agent Memory Register — v1.20.9 M1–M2. A read-only provenance ledger:
//! "who wrote every memory, and what it is based on." The register reads the
//! already-shipped `GET /export` knowledge body and the `GET /get/{id}` wire
//! (no new routes, no new wire types, no writes — this panel never mutates).
//!
//! Pure cores (`register_filter` / `origin_group` / `register_excerpt` /
//! `format_epoch`) are Dioxus-free and unit-tested. The panel is a thin
//! composition over them + the shared `EvidenceModal`, which is also the
//! reusable evidence detail renderer (register rows open it; a recall hit
//! entry is a documented ceiling — recall hits already open the shared drawer).

use crate::api::ApiClient;
use crate::panels::{use_document_title, PageTitle};
use dioxus::prelude::*;

/// One register row — a `/export` knowledge row's provenance projection
/// (the columns the v1.18.2 export already carries: origin/owner/source/kind).
#[derive(Debug, Clone, PartialEq)]
pub struct RegisterRow {
    pub id: i64,
    pub content: String,
    pub origin: String, // human | model | imported
    pub owner: Option<String>,
    pub source: String,
    pub memory_kind: String,
    pub created_at: Option<i64>, // epoch secs in the export body
}

impl RegisterRow {
    /// Tolerant read of one `/export` knowledge row. Unknown/absent fields
    /// degrade to ""/None so an older export body still renders.
    pub fn from_export(v: &serde_json::Value) -> Option<Self> {
        Some(RegisterRow {
            id: v.get("id")?.as_i64()?,
            content: v.get("content")?.as_str()?.to_string(),
            origin: v
                .get("origin")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            owner: v.get("owner").and_then(|x| x.as_str()).map(String::from),
            source: v
                .get("source")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            memory_kind: v
                .get("memory_kind")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            created_at: v.get("created_at").and_then(|x| x.as_i64()),
        })
    }
}

/// Parse the `/export` `knowledge` array into rows. A body without that array
/// (any other endpoint's response) yields zero rows — the register only ever
/// reads the export shape.
pub fn parse_export_rows(body: &serde_json::Value) -> Vec<RegisterRow> {
    body.get("knowledge")
        .and_then(|k| k.as_array())
        .map(|arr| arr.iter().filter_map(RegisterRow::from_export).collect())
        .unwrap_or_default()
}

/// The filter projection — pure. An empty filter is a pass-through; each
/// non-empty filter narrows. Returns a new Vec, never borrows the caller.
pub fn register_filter(
    rows: &[RegisterRow],
    origin: &str,
    owner: &str,
    source: &str,
    kind: &str,
) -> Vec<RegisterRow> {
    rows.iter()
        .filter(|r| origin.is_empty() || r.origin == origin)
        .filter(|r| owner.is_empty() || r.owner.as_deref() == Some(owner))
        .filter(|r| source.is_empty() || r.source == source)
        .filter(|r| kind.is_empty() || r.memory_kind == kind)
        .cloned()
        .collect()
}

/// The Tabs partition — fixed provenance-trust order (human, model, imported)
/// with live counts. Zero-count origins are omitted.
#[derive(Debug, Clone, PartialEq)]
pub struct OriginGroup {
    pub origin: String,
    pub count: usize,
}

pub fn origin_group(rows: &[RegisterRow]) -> Vec<OriginGroup> {
    ["human", "model", "imported"]
        .iter()
        .map(|o| OriginGroup {
            origin: (*o).to_string(),
            count: rows.iter().filter(|r| r.origin == *o).count(),
        })
        .filter(|g| g.count > 0)
        .collect()
}

/// Bounded excerpt at the render boundary: strip invisible smuggling chars
/// first (the v1.20.3 G5 render boundary), then truncate by chars with an
/// ellipsis. `clean` may be shorter than `max` after stripping.
pub fn register_excerpt(content: &str, max: usize) -> String {
    let clean = crate::strip_invisible(content);
    let mut out: String = clean.chars().take(max).collect();
    if clean.chars().count() > max {
        out.push('…');
    }
    out
}

/// Epoch seconds → UTC date `YYYY-MM-DD`. Civil-from-days after Howard
/// Hinnant; keeps the client free of a time dependency. (The `/get/{id}`
/// `created_at` is already an ISO string and is rendered as-is in evidence.)
pub fn format_epoch(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// One evidence row — the `GET /get/{id}` wire shape (already shipped). No
/// new wire type: the register never asks the server for anything it doesn't
/// already return. `highlights`/`source_prompt` are not on this wire; the
/// viewer renders the verbatim span + source link, which is the honest surface.
#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceRow {
    pub id: i64,
    pub title: Option<String>,
    pub content: String,
    pub source_uri: Option<String>,
    pub revision_id: Option<i64>,
    pub heading_path: Option<String>,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub created_at: Option<String>,
}

impl EvidenceRow {
    pub fn from_value(v: &serde_json::Value) -> Option<Self> {
        Some(EvidenceRow {
            id: v.get("id")?.as_i64()?,
            title: v.get("title").and_then(|x| x.as_str()).map(String::from),
            content: v.get("content")?.as_str()?.to_string(),
            source_uri: v
                .get("source_uri")
                .and_then(|x| x.as_str())
                .map(String::from),
            revision_id: v.get("revision_id").and_then(|x| x.as_i64()),
            heading_path: v
                .get("heading_path")
                .and_then(|x| x.as_str())
                .map(String::from),
            line_start: v.get("line_start").and_then(|x| x.as_i64()),
            line_end: v.get("line_end").and_then(|x| x.as_i64()),
            created_at: v
                .get("created_at")
                .and_then(|x| x.as_str())
                .map(String::from),
        })
    }
}

/// The register panel (v1.20.9 M1): reads `/export`, filters client-side,
/// and lets an operator open the evidence detail for any row.
pub fn panel() -> Element {
    use_document_title(|| format!("{} — brain", crate::i18n::t("register_title")));
    let api = use_context::<Signal<ApiClient>>();
    let mut tab = use_signal(String::new); // "" = All
    let mut owner = use_signal(String::new);
    let mut source = use_signal(String::new);
    let mut kind = use_signal(String::new);
    let mut open = use_signal(|| None::<i64>);

    // The register only ever issues a GET /export — the read side of the
    // ledger. Nothing here proposes/purges/approves (see `register_is_read_only`).
    let rows = use_resource(move || {
        let api = api();
        async move {
            api.export("json")
                .await
                .map(|body| parse_export_rows(&body))
        }
    });

    let groups = match &*rows.read() {
        Some(Ok(r)) => origin_group(r),
        _ => Vec::new(),
    };
    let visible = match &*rows.read() {
        Some(Ok(r)) => register_filter(r, &tab(), &owner(), &source(), &kind()),
        _ => Vec::new(),
    };

    let register_owner_ph = crate::i18n::t("register_owner_ph");
    let register_source_ph = crate::i18n::t("register_source_ph");
    let register_kind_ph = crate::i18n::t("register_kind_ph");

    rsx! {
        PageTitle { {crate::i18n::t("register_title")} }
        p { class: "text-muted-foreground text-sm mb-3", {crate::i18n::t("register_sub")} }
        div { class: "card",
            div { class: "card-body",
                // Origin Tabs (provenance-trust order) + filters.
                div { class: "flex flex-wrap gap-2 items-center mb-3",
                    button {
                        class: if tab().is_empty() { "btn btn-sm btn-primary" } else { "btn btn-sm btn-outline" },  // i18n-exempt: css class expression
                        onclick: move |_| tab.set(String::new()),
                        {crate::i18n::t("register_all")}
                    }
                    for g in &groups {
                        button {
                            class: if tab() == g.origin { "btn btn-sm btn-primary" } else { "btn btn-sm btn-outline" },  // i18n-exempt: css class expression
                            onclick: {
                                let o = g.origin.clone();
                                move |_| tab.set(o.clone())
                            },
                            "{g.origin} ({g.count})"
                        }
                    }
                }
                div { class: "flex flex-wrap gap-2 mb-3",
                    input { class: "input", placeholder: "{register_owner_ph}", value: "{owner}", oninput: move |e| owner.set(e.value()) }
                    input { class: "input", placeholder: "{register_source_ph}", value: "{source}", oninput: move |e| source.set(e.value()) }
                    input { class: "input", placeholder: "{register_kind_ph}", value: "{kind}", oninput: move |e| kind.set(e.value()) }
                }
                match &*rows.read() {
                    Some(Err(e)) => rsx! {
                        p { class: "text-danger text-sm", {crate::i18n::t_fmt("register_failed", &[crate::api::error_message(e)])} }
                    },
                    Some(Ok(_)) if visible.is_empty() => rsx! {
                        p { class: "text-muted-foreground text-sm", {crate::i18n::t("register_empty")} }
                    },
                    _ => rsx! {
                        ul { class: "divide-y divide-border",
                            for r in &visible {
                                li { class: "py-2 flex items-start gap-3",
                                    span { class: "font-mono text-xs text-muted-foreground shrink-0", "#{r.id}" }
                                    div { class: "min-w-0 flex-1",
                                        p { class: "text-sm", "{register_excerpt(&r.content, 120)}" }
                                        div { class: "flex flex-wrap gap-2 text-xs mt-0.5",
                                            span { class: "badge", "{r.origin}" }
                                            if let Some(o) = &r.owner { span { class: "badge", {crate::i18n::t_fmt("register_owner", std::slice::from_ref(o))} } }
                                            span { class: "badge", "{r.memory_kind}" }
                                            if !r.source.is_empty() { span { class: "badge", "{r.source}" } }
                                        }
                                    }
                                    div { class: "flex items-center gap-3 shrink-0",
                                        span { class: "text-xs text-muted-foreground tabular",
                                            { r.created_at.map(format_epoch).unwrap_or_default() }
                                        }
                                        button {
                                            class: "btn btn-outline btn-xs",
                                            onclick: { let id = r.id; move |_| open.set(Some(id)) },
                                            {crate::i18n::t("register_evidence")}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        EvidenceModal { open }
    }
}

/// The evidence detail viewer (v1.20.9 M2): "one renderer" — opens from any
/// register row's evidence button. Renders the verbatim chunk span + source
/// link + revision + heading + lines. Reuses the existing `/get/{id}` wire.
#[component]
pub fn EvidenceModal(open: Signal<Option<i64>>) -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let id = match *open.read() {
        Some(id) => id,
        None => return rsx! {},
    };
    let ev = use_resource(move || {
        let api = api();
        async move {
            let v = api.get_raw(&format!("/get/{id}")).await?;
            EvidenceRow::from_value(&v)
                .ok_or_else(|| crate::api::ApiError::Status(404, "no chunk with that id".into()))
        }
    });
    rsx! {
        crate::Modal {
            label: crate::i18n::t("register_evidence_modal").to_string(),
            trap: ".evidence-modal".to_string(),
            initial_focus: ".evidence-modal button".to_string(),
            on_close: move |_| open.set(None),
            div { class: "evidence-modal card p-4 w-full max-w-2xl bg-popover max-h-[80vh] overflow-y-auto",
                div { class: "flex items-center justify-between",
                    h2 { class: "text-lg font-semibold", {crate::i18n::t_fmt("register_evidence_title", &[id.to_string()])} }
                    button { class: "btn btn-ghost btn-md", onclick: move |_| open.set(None), "×" }
                }
                match &*ev.read() {
                    Some(Ok(e)) => rsx! {
                        if let Some(t) = &e.title { p { class: "text-sm text-muted-foreground mt-1", "{t}" } }
                        div { class: "flex flex-wrap gap-2 text-xs mt-2",
                            if let Some(uri) = &e.source_uri { span { class: "badge", {crate::i18n::t_fmt("register_src", std::slice::from_ref(uri))} } }
                            if let Some(rid) = e.revision_id { span { class: "badge", {crate::i18n::t_fmt("register_rev", &[rid.to_string()])} } }
                            if let Some(hp) = &e.heading_path { span { class: "badge", "{hp}" } }
                            if let (Some(a), Some(b)) = (e.line_start, e.line_end) { span { class: "badge", {crate::i18n::t_fmt("register_lines", &[a.to_string(), b.to_string()])} } }
                            if let Some(c) = &e.created_at { span { class: "badge", "{c}" } }
                        }
                        pre { class: "mt-3 p-3 bg-muted/50 rounded font-mono text-xs whitespace-pre-wrap", "{crate::strip_invisible(&e.content)}" }
                    },
                    Some(Err(err)) => rsx! {
                        p { class: "text-danger mt-3", {crate::i18n::t_fmt("register_ev_failed", &[crate::api::error_message(err)])} }
                    },
                    None => rsx! {
                        p { class: "text-muted-foreground mt-3", {crate::i18n::t("register_ev_loading")} }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(id: i64, origin: &str, owner: Option<&str>, source: &str, kind: &str) -> RegisterRow {
        RegisterRow {
            id,
            content: format!("memory {id}"),
            origin: origin.into(),
            owner: owner.map(String::from),
            source: source.into(),
            memory_kind: kind.into(),
            created_at: Some(1_735_689_600), // 2025-01-01
        }
    }

    #[test]
    fn register_filter_filters_by_origin_owner_source_kind() {
        let rows = vec![
            row(1, "human", Some("alice"), "manual", "fact"),
            row(2, "model", Some("agent-7"), "pdf", "rule"),
            row(3, "human", Some("bob"), "manual", "fact"),
            row(4, "imported", Some("alice"), "csv", "fact"),
        ];
        assert_eq!(register_filter(&rows, "", "", "", ""), rows);
        let got = register_filter(&rows, "human", "", "", "");
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|r| r.origin == "human"));
        assert_eq!(
            register_filter(&rows, "human", "alice", "manual", "fact").len(),
            1
        );
        assert_eq!(register_filter(&rows, "", "", "pdf", "").len(), 1);
        assert_eq!(register_filter(&rows, "", "", "", "fact").len(), 3);
        assert!(register_filter(&rows, "human", "nobody", "", "").is_empty());
    }

    #[test]
    fn origin_group_partitions_with_counts() {
        let rows = vec![
            row(1, "human", None, "", "fact"),
            row(2, "model", None, "", "fact"),
            row(3, "human", None, "", "rule"),
        ];
        assert_eq!(
            origin_group(&rows),
            vec![
                OriginGroup {
                    origin: "human".into(),
                    count: 2
                },
                OriginGroup {
                    origin: "model".into(),
                    count: 1
                },
            ]
        );
        assert!(!origin_group(&rows).iter().any(|g| g.origin == "imported"));
    }

    #[test]
    fn register_excerpt_is_bounded_and_strips_invisible() {
        assert_eq!(register_excerpt("héllo", 10), "héllo");
        let t = format!("a{}b{}c", '\u{200B}', '\u{FEFF}');
        assert_eq!(register_excerpt(&t, 10), "abc");
        let long = "x".repeat(200);
        let e = register_excerpt(&long, 50);
        assert!(e.ends_with('…'));
        assert_eq!(e.chars().count(), 51);
    }

    #[test]
    fn evidence_modal_uses_existing_get_route() {
        // the /get/{id} wire shape parses straight into EvidenceRow (no new wire type)
        let v = json!({
            "id": 7, "title": "T", "content": "body",
            "source_uri": "file://a.md", "revision_id": 3,
            "heading_path": "h1", "line_start": 1, "line_end": 4,
            "created_at": "2025-01-01"
        });
        let e = EvidenceRow::from_value(&v).unwrap();
        assert_eq!(e.id, 7);
        assert_eq!(e.source_uri.as_deref(), Some("file://a.md"));
        assert_eq!(e.revision_id, Some(3));
        assert_eq!((e.line_start, e.line_end), (Some(1), Some(4)));
        assert!(EvidenceRow::from_value(&json!({})).is_none());
    }

    #[test]
    fn register_is_read_only() {
        // the register only ever reads the /export `knowledge` body — any
        // write endpoint's response (purge/delete/approve) carries no
        // `knowledge` array and yields zero rows, so the ledger can't be fed
        // a mutation's result.
        assert!(parse_export_rows(&json!({ "ok": true, "purged": 5 })).is_empty());
        let body = json!({
            "knowledge": [
                { "id": 1, "content": "c", "origin": "human", "source": "manual",
                  "memory_kind": "fact", "owner": "alice" }
            ]
        });
        let rows = parse_export_rows(&body);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].origin, "human");
    }

    #[test]
    fn format_epoch_renders_utc_date() {
        assert_eq!(format_epoch(0), "1970-01-01");
        assert_eq!(format_epoch(1_735_689_600), "2025-01-01");
    }
}
