//! Audit panel — the append-only hash-chain browser (DESIGN §4.5). GET /audit
//! exists today; read events appear when BRAIN_AUDIT_READ_EVENTS=on (v1.15.0 M1).
//!
//! v1.16.0 M7: client-side filters (principal / kind / since) + an export
//! button. The backend `GET /audit` supports `?kind=` server-side, but the
//! principal/since params are a v1.19.0 polish — so this release filters those
//! client-side. Export serializes the (filtered) fetched rows to JSON and
//! triggers a download via `document::eval` (no new server route).

use crate::UiState;
use crate::api::{ApiClient, AuditRow};
use crate::i18n::{t, t_fmt};
use crate::panels::{PageTitle, RefreshButton, use_document_title};
use dioxus::prelude::*;

const PAGE: usize = 100; // page size for the server-side audit pagination (M4)

/// M7: the client-side filter state. `None`/empty = unconstrained on that axis.
#[derive(Clone, Debug, Default, PartialEq)]
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

/// v1.19.0 M2: build an `AuditFilter` from the deep-link query params
/// (`/audit?since=&principal=`). `None`/empty → unconstrained on that axis.
pub fn filter_from_query(since: Option<String>, principal: Option<String>) -> AuditFilter {
    AuditFilter {
        principal: principal.unwrap_or_default(),
        kind: String::new(),
        since: since.unwrap_or_default(),
    }
}

/// v1.20.20 M2: the replay deep-link target for an audit row. Only `recall`
/// rows record a decision-path trace, and the audit row id *is* the trace id
/// (v1.15.0 M2 — `read_trace(audit_id)`), so `/recall/{id}` is the replay view.
/// Every other kind returns `None` and stays unlinked. Pure + test-pinned so a
/// future kind that starts recording a trace is never silently left without a link.
pub fn replay_href(kind: &str, id: i64) -> Option<String> {
    if kind == "recall" {
        Some(format!("/recall/{id}"))
    } else {
        None
    }
}

pub fn panel(since: Option<String>, principal: Option<String>) -> Element {
    use_document_title(|| "Audit — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let writes = (ui.writes_enabled)();
    let mut filter = use_signal(move || filter_from_query(since, principal));
    // v1.16.7 M4: server-side pagination. Accumulate pages (newest-first); the
    // first page loads here, "Load more" appends the next offset until the
    // server returns a short page (no more rows).
    let mut events = use_signal(Vec::<AuditRow>::new);
    let mut offset = use_signal(|| 0usize);
    let mut has_more = use_signal(|| true);
    let mut page_err = use_signal(|| None::<String>);
    let refresh = use_signal(|| 0u32);

    // Keep the first-page resource alive (not `let _`, which clippy flags as a
    // dropped future); a named binding holds it across re-renders.
    let _first_page = use_resource(move || {
        let api = api();
        let _ = refresh();
        async move {
            match api.audit_page(0, PAGE).await {
                Ok(resp) => {
                    events.set(resp.events.clone());
                    has_more.set(resp.events.len() >= PAGE);
                    offset.set(resp.events.len());
                    page_err.set(None);
                }
                Err(e) => page_err.set(Some(e.to_string())),
            }
        }
    });

    let load_more = move |_| {
        let api = api();
        let off = offset();
        spawn(async move {
            match api.audit_page(off, PAGE).await {
                Ok(resp) => {
                    let mut all = events();
                    let mut next = resp.events;
                    let tail = all.last().map(|r| r.id);
                    // Server pages are newest-first and disjoint; guard against
                    // a stray duplicate boundary row so ids never repeat.
                    if let Some(tid) = tail {
                        next.retain(|r| r.id < tid);
                    }
                    let n = next.len();
                    all.extend(next);
                    events.set(all);
                    has_more.set(n >= PAGE);
                    offset.set(off + n);
                    page_err.set(None);
                }
                Err(e) => page_err.set(Some(e.to_string())),
            }
        });
    };

    // The distinct kinds present in the data drive the kind dropdown (so the
    // filter reflects what's actually there, not a hardcoded list).
    let kinds: Vec<String> = {
        let mut k: Vec<String> = events().iter().map(|r| r.kind.clone()).collect();
        k.sort();
        k.dedup();
        k
    };

    let rows = filter_audit(&events(), &filter());
    let replay_link = crate::i18n::t("replay_audit_link");

    rsx! {
        PageTitle { {crate::i18n::t("audit_title")} }
        div { class: "card mt-2",
            div { class: "card-body",
                match (events().is_empty(), &page_err()) {
                    (_, Some(e)) => rsx! { p { class: "text-danger text-sm", {t_fmt("audit_error", std::slice::from_ref(e))} } },
                    (true, None) => rsx! { p { class: "text-muted-foreground text-sm", {t("audit_empty")} } },
                    (false, None) => rsx! {
                        p { class: "text-muted-foreground text-sm",
                            {t_fmt("audit_filtered_summary", &[events().len().to_string(), rows.len().to_string()])} }
                    },
                }
                // M7.1: client-side filter controls.
                div { class: "flex gap-2 my-3 flex-wrap items-center",
                    input {
                        class: "input",
                        placeholder: t("audit_principal_placeholder"),
                        value: "{filter().principal}",
                        oninput: move |e| filter.write().principal = e.value(),
                        "aria-label": t("audit_filter_principal"),
                    }
                    select {
                        class: "select",
                        value: "{filter().kind}",
                        onchange: move |e| filter.write().kind = e.value(),
                        "aria-label": t("audit_filter_kind"),
                        option { value: "", {t("audit_all_kinds")} }
                        for k in &kinds {
                            option { value: "{k}", "{k}" }
                        }
                    }
                    input {
                        class: "input",
                        "type": "date",
                        value: "{filter().since}",
                        oninput: move |e| filter.write().since = e.value(),
                        "aria-label": t("audit_filter_since"),
                    }
                    // M7.1: export the filtered rows as JSON. Ponytail: no `/audit/export`
                    // server route exists and "the client adds no new server routes" — so
                    // we serialize the already-fetched rows client-side + trigger a
                    // download via eval (web) / no-op where eval is unavailable.
                        button {
                            class: "btn btn-outline btn-md ml-auto",
                            disabled: !writes || rows.is_empty(),
                            onclick: move |_| {
                                let payload = serde_json::json!({ "events": &rows });
                                let s = payload.to_string();
                                // The one download seam: blob save on web,
                                // native file write on desktop/mobile.
                                let _ = crate::download::save_file("audit.json", &s);
                            },
                            {t("audit_export")}
                        }
                        // v1.17.0 M2.4: portable refresh (resets pagination to page 0).
                        div { class: "ml-1", RefreshButton { refresh } }
                    // M7.5: announce the export to screen readers.
                    if !rows.is_empty() {
                        span { class: "sr-only", role: "status", "aria-live": "polite",
                            {t_fmt("audit_rows_exported", &[rows.len().to_string()])}
                        }
                    }
                }
                // F-59 (v1.27.20 "Console"): the table lives INSIDE the
                // horizontal-scroll wrapper — the pre-fix markup had an empty
                // self-closing `div.overflow-x-auto` beside a bare `<table>`,
                // so a wide kind/actor column overflowed the card on narrow
                // viewports instead of scrolling.
                div { class: "overflow-x-auto",
                    table { class: "table",
                    thead {
                        tr {
                            // i18n-exempt: wire-vocabulary column headers (the
                            // audit row fields as documented in openapi.yaml —
                            // translating them would break ops cross-checks
                            // against the API).
                            th { class: "text-left pr-2", "id" }
                            th { class: "text-left pr-2", "ts" }
                            th { class: "text-left pr-2", "kind" }
                            th { class: "text-left pr-2", "actor" }
                            th { class: "text-left pr-2", "status" }
                            th { class: "text-left", "target_hash" }
                            th { class: "text-left pr-2", "replay" }
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
                                td { class: "pr-2",
                                    if let Some(href) = replay_href(&row.kind, row.id) {
                                        Link { to: href, "{replay_link} ↗" }
                                    }
                                }
                            }
                        }
                    }
                    }
                }
                // v1.16.7 M4: paginated tail. Load the next page (append) until
                // the server returns a short page. A real scroll-detect needs
                // viewport JS; a button is the honest no-JS equivalent.
                if has_more() {
                    div { class: "mt-3 text-center",
                        button {
                            class: "btn btn-outline btn-md",
                            onclick: load_more,
                            {t_fmt("audit_load_more", &[events().len().to_string()])}
                        }
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
    fn filter_from_query_seeds_deep_link_params() {
        // Both params present → both filter axes seeded.
        let f = filter_from_query(Some("2026-08-01".into()), Some("alice".into()));
        assert_eq!(f.since, "2026-08-01");
        assert_eq!(f.principal, "alice");
        assert!(f.kind.is_empty());
        // Absent → unconstrained (kind never comes from the query string).
        let f = filter_from_query(None, None);
        assert_eq!(f, AuditFilter::default());
    }

    #[test]
    fn ts_on_or_after_handles_date_prefix() {
        assert!(ts_on_or_after("2026-08-08T12:00:00Z", "2026-08-08"));
        assert!(ts_on_or_after("2026-08-09T00:00:00Z", "2026-08-08"));
        assert!(!ts_on_or_after("2026-07-31T23:59:59Z", "2026-08-01"));
    }

    /// v1.16.7 M4: merging the next page never repeats a boundary id. A server
    /// that (defensively) returned an overlapping boundary row is deduped.
    #[test]
    fn merge_page_never_repeats_boundary_id() {
        let page0 = vec![
            row(4, "auth", "a", "ok", "t4"),
            row(3, "auth", "a", "ok", "t3"),
        ];
        let page1 = vec![
            row(3, "auth", "a", "ok", "t3"),
            row(2, "auth", "a", "ok", "t2"),
        ];
        let mut all = page0;
        let tid = all.last().map(|r| r.id).unwrap();
        let mut next = page1;
        next.retain(|r| r.id < tid);
        all.extend(next);
        assert_eq!(all.iter().map(|r| r.id).collect::<Vec<_>>(), vec![4, 3, 2]);
    }

    /// v1.20.20 M2: only `recall` audit rows carry a replayable decision path,
    /// and the target is `/recall/{id}` (the audit row id IS the trace id).
    /// Every other kind stays unlinked so a future trace-capable kind must be
    /// wired explicitly rather than silently linking to a missing trace.
    #[test]
    fn replay_href_links_only_recall_rows() {
        assert_eq!(replay_href("recall", 7), Some("/recall/7".to_string()));
        for kind in ["auth", "ingest", "propose", "suggest", "dsar", ""] {
            assert_eq!(replay_href(kind, 7), None, "kind {kind} must not link");
        }
    }
}
