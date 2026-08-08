//! Audit panel — the append-only hash-chain browser (DESIGN §4.5). GET /audit
//! exists today; read events appear when BRAIN_AUDIT_READ_EVENTS=on (v1.15.0 M1).
//!
//! v1.16.0 M7: client-side filters (principal / kind / since) + an export
//! button. The backend `GET /audit` supports `?kind=` server-side, but the
//! principal/since params are a v1.19.0 polish — so this release filters those
//! client-side. Export serializes the (filtered) fetched rows to JSON and
//! triggers a download via `document::eval` (no new server route).

use crate::api::{ApiClient, AuditRow};
use crate::panels::{use_document_title, PageTitle};
use crate::UiState;
use dioxus::prelude::*;

const MAX_ROWS: usize = 100; // mirrors the backend's default audit limit

/// M7: the client-side filter state. `None`/empty = unconstrained on that axis.
#[derive(Clone, Default, PartialEq)]
pub struct AuditFilter {
    principal: String,
    kind: String,
    since: String, // YYYY-MM-DD
}

/// M7 pure: filter audit rows by principal (substring, case-insensitive), kind
/// (exact), and since (ts >= that date). Extracted so the panel is plumbing.
pub fn filter_audit(rows: &[AuditRow], filter: &AuditFilter) -> Vec<AuditRow> {
    rows.iter()
        .filter(|r| {
            if !filter.principal.is_empty()
                && !r
                    .actor
                    .to_lowercase()
                    .contains(&filter.principal.to_lowercase())
            {
                return false;
            }
            if !filter.kind.is_empty() && r.kind != filter.kind {
                return false;
            }
            if !filter.since.is_empty() && !ts_on_or_after(&r.ts, &filter.since) {
                return false;
            }
            true
        })
        .cloned()
        .collect()
}

/// Ponytail: a lexicographic prefix check on the ISO-8601 timestamp against a
/// `YYYY-MM-DD` date. Robust enough for the audit `ts` format the server emits
/// (`2026-08-08T…`); avoids pulling `chrono` for one comparison.
fn ts_on_or_after(ts: &str, date: &str) -> bool {
    // Compare the date prefix (first 10 chars = YYYY-MM-DD) lexicographically.
    ts.get(..10).map(|p| p >= date).unwrap_or(false)
}

pub fn panel() -> Element {
    use_document_title(|| "Audit — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let writes = (ui.writes_enabled)();
    let audit = use_resource(move || {
        let api = api();
        async move { api.audit().await }
    });
    let mut filter = use_signal(AuditFilter::default);

    // The distinct kinds present in the data drive the kind dropdown (so the
    // filter reflects what's actually there, not a hardcoded list).
    let kinds: Vec<String> = match &*audit.read() {
        Some(Ok(resp)) => {
            let mut k: Vec<String> = resp.events.iter().map(|r| r.kind.clone()).collect();
            k.sort();
            k.dedup();
            k
        }
        _ => Vec::new(),
    };

    let rows = match &*audit.read() {
        Some(Ok(resp)) => filter_audit(
            resp.events
                .iter()
                .take(MAX_ROWS)
                .cloned()
                .collect::<Vec<_>>()
                .as_slice(),
            &filter(),
        ),
        _ => Vec::new(),
    };

    rsx! {
        PageTitle { "Audit" }
        match &*audit.read() {
            Some(Ok(resp)) => rsx! {
                p { class: "text-ink-muted text-sm",
                    "{resp.events.len()} events loaded · {rows.len()} after filter (hash-only — no raw content)" }
            },
            _ => rsx! {},
        }
        // M7.1: client-side filter controls.
        div { class: "flex gap-2 my-2 flex-wrap items-center text-sm",
            input {
                class: "border border-border-subtle surface-raised rounded px-2 py-1",
                placeholder: "principal…",
                value: "{filter().principal}",
                oninput: move |e| filter.write().principal = e.value(),
                "aria-label": "filter by principal",
            }
            select {
                class: "border border-border-subtle surface-raised rounded px-1 py-1",
                value: "{filter().kind}",
                onchange: move |e| filter.write().kind = e.value(),
                "aria-label": "filter by kind",
                option { value: "", "all kinds" }
                for k in &kinds {
                    option { value: "{k}", "{k}" }
                }
            }
            input {
                class: "border border-border-subtle surface-raised rounded px-2 py-1",
                "type": "date",
                value: "{filter().since}",
                oninput: move |e| filter.write().since = e.value(),
                "aria-label": "filter since date",
            }
            // M7.1: export the filtered rows as JSON. Ponytail: no `/audit/export`
            // server route exists and "the client adds no new server routes" — so
            // we serialize the already-fetched rows client-side + trigger a
            // download via eval (web) / no-op where eval is unavailable.
            button {
                class: "border border-border-subtle surface-raised rounded px-2 py-1 disabled:opacity-50",
                disabled: !writes || rows.is_empty(),
                onclick: move |_| {
                    let payload = serde_json::json!({ "events": &rows });
                    let s = payload.to_string();
                    // Web: build a blob URL + click it. Desktop: same webview JS.
                    // ponytail: untestable without `dx serve`; the fallback (no-op
                    // on a renderer without JS) is acceptable — the data is still
                    // visible in the table.
                    let js = format!(
                        "(function(){{var b=new Blob([{s:?}],{{type:'application/json'}});var u=URL.createObjectURL(b);var a=document.createElement('a');a.href=u;a.download='audit.json';a.click();URL.revokeObjectURL(u);}})();"
                    );
                    let _ = document::eval(&js);
                },
                "Export JSON"
            }
        }
        div { class: "overflow-x-auto mt-2" }
        table { class: "w-full text-sm tabular",
            thead {
                tr {
                    th { class: "text-left pr-2", "id" }
                    th { class: "text-left pr-2", "ts" }
                    th { class: "text-left pr-2", "kind" }
                    th { class: "text-left pr-2", "actor" }
                    th { class: "text-left pr-2", "status" }
                    th { class: "text-left", "target_hash" }
                }
            }
            tbody {
                for row in &rows {
                    tr {
                        td { class: "pr-2 font-mono", "{row.id}" }
                        td { class: "pr-2 whitespace-nowrap", "{row.ts}" }
                        td { class: "pr-2", "{row.kind}" }
                        td { class: "pr-2 font-mono text-xs", "{row.actor}" }
                        td { class: "pr-2",
                            span { class: status_class(&row.status), "{row.status}" }
                        }
                        td { class: "font-mono text-xs", "{row.target_hash}" }
                    }
                }
            }
        }
    }
}

fn status_class(status: &str) -> &'static str {
    match status {
        "ok" => "text-ok",
        "denied" => "text-warn",
        "error" => "text-danger",
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// M7 tests — the client-side filter across all three dimensions.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, kind: &str, actor: &str, status: &str, ts: &str) -> AuditRow {
        AuditRow {
            id,
            ts: ts.into(),
            kind: kind.into(),
            actor: actor.into(),
            target_hash: "h".into(),
            status: status.into(),
            detail_hash: String::new(),
            tenant_id: String::new(),
        }
    }

    #[test]
    fn filter_audit_filters_by_kind_and_principal_and_since() {
        let rows = vec![
            row(1, "auth", "cli", "denied", "2026-08-01T00:00:00Z"),
            row(2, "recall", "user:alice", "ok", "2026-08-05T00:00:00Z"),
            row(3, "auth", "user:bob", "ok", "2026-08-08T00:00:00Z"),
            row(4, "ingest", "user:alice", "ok", "2026-07-30T00:00:00Z"),
        ];
        // Kind filter.
        let auth_only = filter_audit(
            &rows,
            &AuditFilter {
                kind: "auth".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            auth_only.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![1, 3]
        );
        // Principal substring (case-insensitive).
        let alice = filter_audit(
            &rows,
            &AuditFilter {
                principal: "ALICE".into(),
                ..Default::default()
            },
        );
        assert_eq!(alice.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2, 4]);
        // Since date.
        let august = filter_audit(
            &rows,
            &AuditFilter {
                since: "2026-08-01".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            august.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        // Combined.
        let combined = filter_audit(
            &rows,
            &AuditFilter {
                principal: "alice".into(),
                since: "2026-08-01".into(),
                ..Default::default()
            },
        );
        assert_eq!(combined.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2]);
        // Empty filter = all rows.
        assert_eq!(
            filter_audit(&rows, &AuditFilter::default()).len(),
            rows.len()
        );
    }

    #[test]
    fn ts_on_or_after_handles_date_prefix() {
        assert!(ts_on_or_after("2026-08-08T12:00:00Z", "2026-08-08"));
        assert!(ts_on_or_after("2026-08-09T00:00:00Z", "2026-08-08"));
        assert!(!ts_on_or_after("2026-07-31T23:59:59Z", "2026-08-01"));
    }
}
