//! v1.27.11 "Console" — the BPO dashboard views.
//!
//! Two read-only panels over the R1–R9 register API, role-gated by the R9
//! presets + `role::console_view`:
//!   1. `client_admin()` — the `client-auditor` single-client dashboard. It
//!      renders ONLY the clients the JWT grants via the client-side allowlist
//!      mirror of the server `client_authorized_domains` seam (defense-in-
//!      depth: even a misconfigured server response never renders a foreign
//!      client), and it has NO client switcher — the honest single-tenant-per-
//!      client poster.
//!   2. `bpo_ops()` — the `bpo-ops`/admin all-clients board: per-client rows
//!      + connector sync + review-queue depth.
//!
//! Both are read-only (their roles `can == ["read"]`): the panels fetch, they
//! never mutate, and every endpoint the server gates stays server-enforced.

use crate::api::{error_message, ApiClient, ClientRow, ConnectorRow};
use crate::panels::{use_document_title, PageTitle};
use dioxus::prelude::*;

/// v1.27.11 M3: the client-side re-filter. `allowlist = None` (not a client-
/// auditor, or no token) passes every row through — the server returned it.
/// `Some(list)` keeps only rows whose `domain` is in the allowlist; `Some([])`
/// renders nothing (deny-by-default). This is the "never renders foreign
/// clients" guarantee, independent of server behavior.
pub fn filter_granted<'a>(
    rows: &'a [ClientRow],
    allowlist: Option<&[String]>,
) -> Vec<&'a ClientRow> {
    match allowlist {
        None => rows.iter().collect(),
        Some(al) => rows
            .iter()
            .filter(|c| al.iter().any(|d| d == &c.domain))
            .collect(),
    }
}

/// The connector state color token for a row's status.
pub fn connector_state_cls(state: &str) -> &'static str {
    match state {
        "ok" => "text-ok",
        "error" => "text-danger",
        _ => "text-muted-foreground",
    }
}

/// The panel a role-view resolves to. `Undefined` → `Stock`: a non-BPO
/// principal (agent/staff/no-roles) reaching `/clients` directly (palette or
/// deep link) gets the stock operations console, never the all-clients BPO
/// board. Pure so the security property is unit-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsolePanel {
    ClientAdmin,
    BpoOps,
    Stock,
}

pub fn console_panel(view: crate::role::ConsoleView) -> ConsolePanel {
    match view {
        crate::role::ConsoleView::ClientAdmin => ConsolePanel::ClientAdmin,
        crate::role::ConsoleView::BpoOps => ConsolePanel::BpoOps,
        crate::role::ConsoleView::Undefined => ConsolePanel::Stock,
    }
}

pub fn panel() -> Element {
    let api = use_context::<Signal<ApiClient>>();
    match console_panel(crate::role::console_view(&api().roles())) {
        ConsolePanel::ClientAdmin => client_admin(),
        ConsolePanel::BpoOps => bpo_ops(),
        ConsolePanel::Stock => crate::panels::ops::panel(),
    }
}

fn client_admin() -> Element {
    use_document_title(|| "Client dashboard — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let clients = use_resource(move || {
        let api = api();
        async move { api.clients().await }
    });

    // v1.27.11 M3: the auditor's client-domain allowlist (the client mirror of
    // the server `client_authorized_domains` seam). Computed fresh each render
    // from the live token so a reconnect/roles change is picked up.
    let granted = api().client_auditor_domains();
    let shown: Vec<ClientRow> = match &*clients.read() {
        Some(Ok(rows)) => filter_granted(rows, granted.as_deref())
            .into_iter()
            .cloned()
            .collect(),
        _ => Vec::new(),
    };

    rsx! {
        PageTitle { {crate::i18n::t("console_client_title")} }
        p { class: "text-sm text-muted-foreground", {crate::i18n::t("console_client_sub")} }
        match &*clients.read() {
            Some(Err(e)) => rsx! { p { class: "text-danger", "clients failed: {error_message(&e)}" } },
            _ => rsx! {
                div { class: "mt-3 grid gap-4 md:grid-cols-2",
                    for c in &shown {
                        ClientCard { client: c.clone() }
                    }
                }
                if shown.is_empty() {
                    div { class: "card mt-3", div { class: "card-body text-ink-faint", {crate::i18n::t("console_empty")} } }
                }
            },
        }
    }
}

#[component]
fn ClientCard(client: ClientRow) -> Element {
    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "card-title flex items-center gap-2",
                    "{client.name}"
                    if client.status == "archived" {
                        span { class: "badge badge-warn", "archived" }
                    }
                }
                span { class: "badge", "{client.jurisdiction}" }
            }
            dl { class: "card-body grid grid-cols-2 gap-x-4 gap-y-1.5 text-sm tabular",
                if !client.domain.is_empty() {
                    dt { class: "text-muted-foreground", "domain" } dd { "{client.domain}" }
                }
                if let Some(p) = &client.profile {
                    dt { class: "text-muted-foreground", "profile" } dd { "{p}" }
                }
                dt { class: "text-muted-foreground", "status" } dd { "{client.status}" }
            }
        }
    }
}

fn bpo_ops() -> Element {
    use_document_title(|| "BPO ops — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let clients = use_resource(move || {
        let api = api();
        async move { api.clients().await }
    });
    let connectors = use_resource(move || {
        let api = api();
        async move {
            api.connectors()
                .await
                .map(|c| c.connectors)
                .unwrap_or_default()
        }
    });
    let pending = use_resource(move || {
        let api = api();
        async move {
            api.proposals("pending")
                .await
                .map(|v| v.len())
                .unwrap_or_default()
        }
    });

    let rows: Vec<ClientRow> = match &*clients.read() {
        Some(Ok(rows)) => rows.clone(),
        _ => Vec::new(),
    };
    let conns: Vec<ConnectorRow> = match &*connectors.read() {
        Some(v) => v.clone(),
        None => Vec::new(),
    };
    let queued_label = crate::i18n::t("console_queue_depth");
    let depth = match &*pending.read() {
        Some(n) => *n,
        None => 0,
    };

    rsx! {
        PageTitle { {crate::i18n::t("console_ops_title")} }
        p { class: "text-sm text-muted-foreground", {crate::i18n::t("console_ops_sub")} }
        div { class: "card mt-3",
            div { class: "card-header",
                div { class: "card-title flex items-center gap-2",
                    {crate::i18n::t("console_ops_board")}
                    span { class: "text-sm text-muted-foreground tabular", "{queued_label}: {depth}" }
                }
            }
            div { class: "card-body space-y-2",
                match &*clients.read() {
                    Some(Err(e)) => rsx! { p { class: "text-danger", "clients failed: {error_message(&e)}" } },
                    _ => rsx! {
                        for c in &rows {
                            div { class: "flex items-center justify-between rounded border border-border p-2 text-sm",
                                div { class: "flex items-center gap-2",
                                    span { class: "font-semibold", "{c.name}" }
                                    span { class: "badge", "{c.jurisdiction}" }
                                    if c.status == "archived" {
                                        span { class: "badge badge-warn", "archived" }
                                    }
                                }
                                div { class: "text-xs text-muted-foreground tabular", "{c.domain}" }
                            }
                        }
                        if rows.is_empty() {
                            div { class: "text-ink-faint text-sm", {crate::i18n::t("console_empty")} }
                        }
                    },
                }
            }
        }
        div { class: "card mt-2",
            div { class: "card-header", div { class: "card-title", {crate::i18n::t("console_connectors")} } }
            div { class: "card-body space-y-1",
                for c in &conns {
                    div { class: "flex items-center justify-between rounded border border-border p-2 text-sm",
                        span { class: "font-mono text-xs", "{c.kind} · {c.instance}" }
                        span { class: "{connector_state_cls(&c.state)} text-xs", "{c.state}" }
                    }
                }
                if conns.is_empty() {
                    div { class: "text-ink-faint text-sm", {crate::i18n::t("console_empty")} }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::role::ConsoleView;

    fn row(name: &str, domain: &str) -> ClientRow {
        ClientRow {
            name: name.into(),
            domain: domain.into(),
            jurisdiction: "us".into(),
            status: "active".into(),
            profile: None,
            created_at: None,
            archived_at: None,
        }
    }

    #[test]
    fn console_panel_maps_each_view_exactly() {
        assert_eq!(
            console_panel(ConsoleView::ClientAdmin),
            ConsolePanel::ClientAdmin
        );
        assert_eq!(console_panel(ConsoleView::BpoOps), ConsolePanel::BpoOps);
        assert_eq!(
            console_panel(ConsoleView::Undefined),
            ConsolePanel::Stock,
            "non-BPO principal never gets the all-clients BPO board"
        );
    }

    #[test]
    fn client_admin_view_never_renders_foreign_clients() {
        // R9 already row-filters the server response; the client re-filters by
        // the token allowlist so even a leak never renders a foreign client.
        let rows = vec![row("acme", "acme-us"), row("beta", "beta-eu")];
        let granted = vec!["acme-us".to_string()];
        let shown = filter_granted(&rows, Some(&granted));
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].domain, "acme-us");
        // An auditor with no granted domain renders nothing (deny-by-default).
        let none: Vec<String> = vec![];
        assert!(filter_granted(&rows, Some(&none)).is_empty());
        // A non-auditor (no allowlist) passes the server's rows through.
        assert_eq!(filter_granted(&rows, None).len(), 2);
    }

    #[test]
    fn connector_state_maps_to_color() {
        assert_eq!(connector_state_cls("ok"), "text-ok");
        assert_eq!(connector_state_cls("error"), "text-danger");
        assert_eq!(connector_state_cls("other"), "text-muted-foreground");
    }
}
