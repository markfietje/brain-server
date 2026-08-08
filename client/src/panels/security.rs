//! Security panel — quarantine review + audit-chain verify (DESIGN §4.4) + the
//! v1.16.0 M6 auth-failure feed. The post-CVE-2026-59726 surface: injection
//! suspects with their source, release / hard-delete, a one-click chain check,
//! and a feed of recent 401/403s that proves the backend isn't the
//! unauthenticated-memory-access class.

use crate::api::{ApiClient, AuditRow};
use crate::panels::{use_document_title, PageTitle};
use crate::UiState;
use dioxus::prelude::*;

/// M6 pure: filter audit rows to denied-auth events (kind == "auth" AND
/// status == "denied"). The backend records auth rejections with this pair;
/// the client isolates them for the feed. Pure so the panel is plumbing.
pub fn auth_failures(rows: &[AuditRow]) -> Vec<AuditRow> {
    rows.iter()
        .filter(|r| r.kind == "auth" && r.status == "denied")
        .cloned()
        .collect()
}

pub fn panel() -> Element {
    use_document_title(|| "Security — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let mut ui = use_context::<UiState>();
    let writes = (ui.writes_enabled)();
    let refresh = use_signal(|| 0u32);
    let mut chain = use_signal(|| None::<Result<bool, String>>);

    let quarantine = use_resource(move || {
        let api = api();
        let _ = refresh();
        async move { api.quarantine().await }
    });

    // M6: the auth-failure feed. `GET /audit?kind=auth` filters server-side;
    // we then isolate `status == "denied"` (the actual rejections).
    let auth = use_resource(move || {
        let api = api();
        async move { api.audit_kind("auth").await }
    });

    let q_count = match &*quarantine.read() {
        Some(Ok(q)) => q.count,
        _ => 0,
    };

    // Publish badges the AppShell rail reads (M2.1).
    use_effect(move || {
        ui.quarantine_count.set(q_count);
    });
    let fail_count = match &*auth.read() {
        Some(Ok(resp)) => auth_failures(&resp.events).len() as u32,
        _ => 0,
    };
    use_effect(move || {
        ui.auth_failures_count.set(fail_count);
    });

    let failures = match &*auth.read() {
        Some(Ok(resp)) => auth_failures(&resp.events),
        _ => Vec::new(),
    };

    rsx! {
        PageTitle { "Security" }
        div { class: "card",
            div { class: "card-header",
                div { class: "card-title", "Audit chain" }
                match &*chain.read() {
                    Some(Ok(true)) => rsx! { span { class: "badge badge-ok", "chain ok" } },
                    Some(Ok(false)) => {
                        ui.audit_dirty.set(true);
                        rsx! { span { class: "badge badge-danger", "CHAIN TAMPERED" } }
                    }
                    Some(Err(e)) => rsx! { span { class: "badge badge-danger", "{e}" } },
                    None => rsx! { span { class: "badge", "the trust anchor" } },
                }
            }
            div { class: "card-body flex items-center gap-3",
                button {
                    class: "btn btn-outline btn-md",
                    disabled: !writes,
                    onclick: move |_| async move {
                        chain.set(Some(verify_chain(api()).await));
                    },
                    "Verify audit chain"
                }
            }
        }
        h2 { class: "mt-4 text-base font-semibold", "Quarantine ({q_count})" }
        match &*quarantine.read() {
            Some(Ok(q)) if !q.quarantined.is_empty() => rsx! {
                ul { class: "mt-2 divide-y divide-border",
                    for row in &q.quarantined {
                        li { class: "py-2.5",
                            div { class: "flex justify-between items-center",
                                span { class: "font-mono text-sm", "chunk #{row.id}" }
                                span { class: "flex gap-2",
                                    button {
                                        class: "btn btn-outline btn-sm",
                                        disabled: !writes,
                                        onclick: { let mut refresh = refresh; let id = row.id; move |_| async move {
                                            let _ = api().quarantine_action(id, "release").await;
                                            refresh += 1;
                                        } },
                                        "Release"
                                    }
                                    button {
                                        class: "btn btn-destructive btn-sm",
                                        disabled: !writes,
                                        onclick: { let mut refresh = refresh; let id = row.id; move |_| async move {
                                            let _ = api().quarantine_action(id, "delete").await;
                                            refresh += 1;
                                        } },
                                        "Delete"
                                    }
                                }
                            }
                            if let Some(src) = &row.source {
                                p { class: "text-xs text-ink-faint mt-0.5", "source: {src}" }
                            }
                            if let Some(h) = &row.content_hash {
                                p { class: "text-xs font-mono text-ink-faint", "{h}" }
                            }
                        }
                    }
                }
            },
            Some(Ok(_)) => rsx! { p { class: "text-muted-foreground mt-2", "no quarantined chunks" } },
            Some(Err(e)) => rsx! { p { class: "text-danger mt-2", "quarantine failed: {e}" } },
            None => rsx! { p { class: "text-muted-foreground mt-2", "…" } },
        }
        // M6: the auth-failure feed — recent 401/403s with principal + route.
        h2 { class: "mt-4 text-base font-semibold", "Auth failures ({failures.len()})" }
        if failures.is_empty() {
            p { class: "text-muted-foreground text-sm mt-1", "no recent denied-auth events" }
        } else {
            div { class: "card mt-2 overflow-x-auto",
                table { class: "table",
                    thead { tr {
                        th { class: "text-left pr-2", "ts" }
                        th { class: "text-left pr-2", "actor" }
                        th { class: "text-left pr-2", "target" }
                        th { class: "text-left", "status" }
                    } }
                    tbody {
                        for r in &failures {
                            tr {
                                td { class: "pr-2 whitespace-nowrap text-xs", "{r.ts}" }
                                td { class: "pr-2 font-mono text-xs", "{r.actor}" }
                                td { class: "pr-2 font-mono text-xs", "{r.target_hash}" }
                                td { class: "pr-2 text-danger", "{r.status}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn verify_chain(api: ApiClient) -> Result<bool, String> {
    match api.audit_verify().await {
        Ok(v) => Ok(v.ok),
        Err(e) => Err(format!("chain verify failed: {e}")),
    }
}

// ---------------------------------------------------------------------------
// M6 tests — the auth-failure feed parses denied rows.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, kind: &str, status: &str) -> AuditRow {
        AuditRow {
            id,
            ts: "2026-08-08T00:00:00Z".into(),
            kind: kind.into(),
            actor: "api".into(),
            target_hash: "/path".into(),
            status: status.into(),
            detail_hash: String::new(),
            tenant_id: String::new(),
        }
    }

    /// The feed isolates denied-auth rows: kind=auth AND status=denied.
    #[test]
    fn auth_failure_feed_parses_denied_rows() {
        let rows = vec![
            row(1, "auth", "denied"),
            row(2, "auth", "ok"),       // successful auth — not a failure
            row(3, "ingest", "denied"), // different kind — not auth
            row(4, "auth", "denied"),
            row(5, "recall", "ok"),
        ];
        let failures = auth_failures(&rows);
        let ids: Vec<i64> = failures.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![1, 4]);
    }
}
