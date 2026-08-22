//! Overview — the decision-first landing page (v1.17.6 M2). A control room,
//! not a widget dump: a status row (health / snapshot / retention / UMP), a
//! DAR-chain alert list (signal → diagnosis → action link), and a queue
//! preview. Backend stays the source of truth — every card re-fetches via
//! `use_resource`; nothing is cached client-side.
//!
//! The alert list is driven by the pure `overview_alerts` (below) so the panel
//! is plumbing and the severity ordering / empty case are testable.

use crate::api::{ApiClient, error_message};
use crate::panels::{PageTitle, RefreshButton, use_document_title};
use crate::{Route, UiState};
use dioxus::prelude::*;

/// Alert severity, ordered low → high (`Ord`). The alert list sorts Danger
/// first so the operator sees the highest-stakes signal top-most.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Info,
    Warn,
    Danger,
}

/// One alert row: a colored signal dot + the count + a link into the panel
/// that owns it. `signal` is an i18n key (rendered via `t()`), `count` the
/// locale-formatted number, `link_route` the deep link.
#[derive(Clone, PartialEq, Debug)]
pub struct Alert {
    pub severity: Severity,
    pub signal: &'static str,
    pub count: u64,
    pub link_route: Route,
}

impl Alert {
    fn new(severity: Severity, signal: &'static str, count: u64, link_route: Route) -> Self {
        Self {
            severity,
            signal,
            count,
            link_route,
        }
    }
}

/// v1.17.6 M2.5 pure: derive the DAR-chain alert list from the counts the panel
/// already holds. Returns alerts sorted by severity (Danger first). Empty when
/// nothing is flagged (the panel then renders "no alerts", not a blank).
/// ponytail: alerts whose owning panel is still future (decayed/tombstones →
/// Rights, conflicts/near-dups/stale → Consolidate, both v1.17.7/v1.17.8) link
/// to Health for now; those re-point when the panels land.
pub fn overview_alerts(
    quarantine: u64,
    decayed: usize,
    auth_fail: u32,
    conflicts: usize,
    near_dups: usize,
    stale: usize,
    tombstones: usize,
) -> Vec<Alert> {
    let mut out: Vec<Alert> = Vec::new();
    if auth_fail > 0 {
        out.push(Alert::new(
            Severity::Danger,
            "alert_auth_failures",
            auth_fail as u64,
            Route::Security {},
        ));
    }
    if quarantine > 0 {
        out.push(Alert::new(
            Severity::Warn,
            "alert_quarantine",
            quarantine,
            Route::Security {},
        ));
    }
    if stale > 0 {
        out.push(Alert::new(
            Severity::Warn,
            "alert_stale_sources",
            stale as u64,
            Route::Health {},
        ));
    }
    if conflicts > 0 {
        out.push(Alert::new(
            Severity::Warn,
            "alert_conflicts",
            conflicts as u64,
            Route::Health {},
        ));
    }
    if decayed > 0 {
        out.push(Alert::new(
            Severity::Info,
            "alert_decayed",
            decayed as u64,
            Route::Health {},
        ));
    }
    if near_dups > 0 {
        out.push(Alert::new(
            Severity::Info,
            "alert_near_duplicates",
            near_dups as u64,
            Route::Health {},
        ));
    }
    if tombstones > 0 {
        out.push(Alert::new(
            Severity::Info,
            "alert_tombstones",
            tombstones as u64,
            Route::Health {},
        ));
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.severity));
    out
}

pub fn panel() -> Element {
    use_document_title(|| "Overview — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let writes = (ui.writes_enabled)();
    let quarantine = (ui.quarantine_count)();
    let auth_fail = (ui.auth_failures_count)();
    let refresh = use_signal(|| 0u32);

    // Localized strings, precomputed so `"{var}"` interpolation in the rsx
    // never nests a `t("…")` call inside a formatted string (the parser hazard
    // Agent 54 caught).
    let view = crate::i18n::t("view");
    let open_queue = crate::i18n::t("open_queue");
    let no_alerts = crate::i18n::t("no_alerts");
    let no_pending = crate::i18n::t("no_pending");
    let approve = crate::i18n::t("approve");
    let reject = crate::i18n::t("reject");
    let alerts_title = crate::i18n::t("overview_alerts");
    let kinds = crate::i18n::t("kinds");

    // M2.2: the four status resources.
    let health = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().health().await }
    });
    let snapshots = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().snapshot_status().await }
    });
    let retention = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().retention().await }
    });
    let ump = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().ump_capabilities().await }
    });
    // M2.3: the alert sources.
    let decayed = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().decayed().await }
    });
    let propose = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().consolidate_propose().await }
    });
    let tombstones = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().tombstones(100).await }
    });
    // M2.4: the queue preview.
    let proposals = use_resource(move || {
        let api = api;
        let _ = refresh();
        async move { api().proposals("pending").await }
    });

    let decayed_n = match &*decayed.read() {
        Some(Ok(v)) => v.len(),
        _ => 0,
    };
    let propose_n = match &*propose.read() {
        Some(Ok(p)) => (
            p.conflicts.len(),
            p.near_duplicates.len(),
            p.stale_sources.len(),
        ),
        _ => (0, 0, 0),
    };
    let tombstones_n = match &*tombstones.read() {
        Some(Ok(t)) => t.tombstones.len(),
        _ => 0,
    };
    let alerts = overview_alerts(
        quarantine,
        decayed_n,
        auth_fail,
        propose_n.0,
        propose_n.1,
        propose_n.2,
        tombstones_n,
    );

    // M2.4: materialize the queue preview into an owned Vec of (id, kind) so
    // the `onclick` closures capture Copy/owned values (a `let` statement or a
    // borrowed row can't live inside a Dioxus `for` body).
    let preview: Vec<(i64, String)> = match &*proposals.read() {
        Some(Ok(list)) => list
            .iter()
            .take(5)
            .map(|p| (p.id, p.kind.clone()))
            .collect(),
        _ => Vec::new(),
    };

    // M2.4: one-click queue actions (mirrors the review panel's decide). The
    // `refresh += 1` happens inside the spawned task, so the closure only
    // copies the Copy signals into the future → it stays `Fn` + `Copy`.
    // v1.27.19 "Scrub" (D-7): a failed decide is surfaced, never swallowed.
    let action_err = use_signal(String::new);
    let decide = move |id: i64, reject: bool| {
        let api = api;
        let mut refresh = refresh;
        let mut action_err = action_err;
        spawn(async move {
            let res: Result<(), crate::api::ApiError> = if reject {
                api().reject_proposal(id, None).await.map(|_| ())
            } else {
                api().approve_proposal(id, None, None).await.map(|_| ())
            };
            if let Err(e) = res {
                action_err.set(crate::api::error_message(&e));
            }
            refresh += 1;
        });
    };

    rsx! {
        PageTitle { {crate::i18n::t("overview_title")} }
        // v1.28.4 M5: the approval control center dock — the HITL queue on the
        // home surface (digest-bound decisions; the server still enforces).
        crate::approvals::ApprovalDock {}
        div { class: "flex justify-end my-2", RefreshButton { refresh } }
        // M2.2 — status row.
        div { class: "mt-2 grid gap-4 md:grid-cols-2",
            match &*health.read() {
                Some(Ok(h)) => rsx! {
                    StatusCard { label: crate::i18n::t("overview_health"), route: Route::Health {} }
                    div { class: "flex items-center gap-2",
                        span { class: "size-2.5 rounded-full bg-ok" }
                        span { class: "font-semibold", "{h.status}" }
                        span { class: "text-muted-foreground", "v{h.version}" }
                    }
                },
                Some(Err(e)) => rsx! { StatusCard { label: crate::i18n::t("overview_health"), route: Route::Health {},
                    p { class: "text-danger text-sm", "{error_message(&e)}" } } },
                None => rsx! { StatusCard { label: crate::i18n::t("overview_health"), route: Route::Health {},
                    p { class: "text-muted-foreground", "…" } } },
            }
            match &*snapshots.read() {
                Some(Ok(s)) => rsx! {
                    StatusCard { label: crate::i18n::t("overview_snapshot"), route: Route::Health {},
                        div { class: "flex items-center gap-2",
                            span { class: if s.all_ok { "size-2.5 rounded-full bg-ok" } else { "size-2.5 rounded-full bg-danger" } }  // i18n-exempt: css class expression
                            span { class: "tabular", "{s.snapshot_count}" }
                        }
                    }
                },
                Some(Err(_)) => rsx! {},
                None => rsx! {},
            }
            match &*retention.read() {
                Some(Ok(r)) => rsx! {
                    StatusCard { label: crate::i18n::t("overview_retention"), route: Route::Health {},
                        div { class: "flex items-center gap-2",
                            span { class: if r.enabled { "size-2.5 rounded-full bg-ok" } else { "size-2.5 rounded-full bg-warn" } }  // i18n-exempt: css class expression
                            span { class: "tabular", "{r.counts.len()}" }
                            span { class: "text-muted-foreground", "{kinds}" }
                        }
                    }
                },
                Some(Err(_)) => rsx! {},
                None => rsx! {},
            }
            match &*ump.read() {
                Some(Ok(u)) => rsx! {
                    StatusCard { label: crate::i18n::t("overview_ump"), route: Route::Health {},
                        div { class: "flex items-center gap-2",
                            span { class: "size-2.5 rounded-full bg-ok" }
                            span { class: "text-muted-foreground", "v{u.server.version}" }
                            span { class: "badge badge-ok", "UMP {u.conformance}" }
                        }
                    }
                },
                Some(Err(_)) => rsx! {},
                None => rsx! {},
            }
        }
        // M2.3 — the alert list (signal + diagnosis + action link).
        div { class: "card mt-4",
            div { class: "card-header", div { class: "card-title", "{alerts_title}" } }
            div { class: "card-body",
                if alerts.is_empty() {
                    p { class: "text-sm text-ok", "{no_alerts}" }
                } else {
                    ul { class: "divide-y divide-border",
                        for a in alerts {
                            li { class: "flex items-center gap-3 py-2",
                                span { class: severity_dot(a.severity) }
                                span { class: "text-sm tabular", "{crate::i18n::format_number(a.count)}" }
                                span { class: "text-sm flex-1", {crate::i18n::t(a.signal)} }
                                Link { to: a.link_route, class: "text-xs text-accent hover:underline", "{view}" }
                            }
                        }
                    }
                }
            }
        }
        // M2.4 — the queue preview.
        div { class: "card mt-4",
            div { class: "card-header flex items-center justify-between",
                div { class: "card-title", {crate::i18n::t("nav_review")} }
                Link { to: Route::Review {}, class: "text-xs text-accent hover:underline", "{open_queue}" }
            }
            div { class: "card-body",
                match &*proposals.read() {
                    Some(Ok(list)) if !list.is_empty() => rsx! {
                        ul { class: "divide-y divide-border",
                            for (pid, kind) in preview {
                                li { class: "flex items-center gap-3 py-2",
                                    Link {
                                        to: Route::ReviewDetail { proposal_id: pid },
                                        class: "font-mono text-sm text-accent hover:underline truncate flex-1",
                                        "proposal #{pid} · {kind}"
                                    }
                                    button {
                                        class: "btn btn-primary btn-sm",
                                        disabled: !writes,
                                        onclick: move |_| decide(pid, false),
                                        "{approve}"
                                    }
                                    button {
                                        class: "btn btn-ghost btn-sm",
                                        disabled: !writes,
                                        onclick: move |_| decide(pid, true),
                                        "{reject}"
                                    }
                                }
                            }
                        }
                    },
                    Some(Ok(_)) => rsx! { p { class: "text-sm text-muted-foreground", "{no_pending}" } },
                    Some(Err(e)) => rsx! { p { class: "text-danger text-sm", "{error_message(&e)}" } },
                    None => rsx! { p { class: "text-muted-foreground", "…" } },
                }
            }
        }
    }
}

/// One status card: a label + a link into its owning panel + children content.
#[component]
fn StatusCard(label: String, route: Route, children: Element) -> Element {
    rsx! {
        div { class: "card",
            div { class: "card-header flex items-center justify-between",
                div { class: "card-title", "{label}" }
                Link { to: route, class: "text-xs text-accent hover:underline", {crate::i18n::t("view")} }
            }
            div { class: "card-body", {children} }
        }
    }
}

fn severity_dot(sev: Severity) -> &'static str {
    match sev {
        Severity::Danger => "size-2.5 rounded-full bg-danger",
        Severity::Warn => "size-2.5 rounded-full bg-warn",
        Severity::Info => "size-2.5 rounded-full bg-info",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The empty case renders as no alerts (the panel shows "no alerts").
    #[test]
    fn overview_alerts_empty_when_nothing_flagged() {
        assert!(overview_alerts(0, 0, 0, 0, 0, 0, 0).is_empty());
    }

    /// Severity ordering: Danger first, then Warn, then Info — the operator
    /// sees the highest-stakes signal top-most.
    #[test]
    fn overview_alerts_sort_by_severity_danger_first() {
        let alerts = overview_alerts(1, 1, 1, 1, 1, 1, 1);
        let sev: Vec<Severity> = alerts.iter().map(|a| a.severity).collect();
        assert_eq!(sev[0], Severity::Danger, "auth failures lead");
        // The rest must be non-increasing (each ≥ the next).
        assert!(sev.windows(2).all(|w| w[0] >= w[1]));
        // Every flagged source produced a row.
        assert_eq!(alerts.len(), 7);
        let by_signal: std::collections::HashMap<_, _> =
            alerts.iter().map(|a| (a.signal, a.count)).collect();
        assert_eq!(by_signal["alert_auth_failures"], 1);
        assert_eq!(by_signal["alert_decayed"], 1);
    }

    /// Only non-zero sources produce a row.
    #[test]
    fn overview_alerts_only_nonzero_sources() {
        let alerts = overview_alerts(3, 0, 0, 0, 0, 2, 0);
        assert_eq!(alerts.len(), 2);
        let by_signal: std::collections::HashMap<_, _> =
            alerts.iter().map(|a| (a.signal, a.count)).collect();
        assert_eq!(by_signal["alert_quarantine"], 3);
        assert_eq!(by_signal["alert_stale_sources"], 2);
    }
}
