//! The approval control center dock: the HITL queue rendered as a
//! `conversation.input.dock` slot entry (order 5 — between plan and queue)
//! instead of a separate page. Approve binds the `content_digest` the operator
//! saw; drift 409s server-side (ReviewArmour). Role-gated in the UI only —
//! the server enforces.

use crate::api::ApiClient;
use crate::i18n::{t, t_fmt};
use crate::role;
use crate::time_budget;
use crate::{Conn, UiState};
use dioxus::prelude::*;

/// The dock slot registration for this surface (pure, testable): approval at
/// order 5, queue badge strip at 20 — third parties insert between.
#[cfg(test)]
pub fn dock_orders() -> Vec<(&'static str, i32)> {
    let mut reg = crate::slots::SlotRegistry::new();
    use crate::slots::slot_names::InputDock;
    reg.register::<InputDock>(crate::slots::SlotSpec::new("approval", 5));
    reg.register::<InputDock>(crate::slots::SlotSpec::new("queue", 20));
    reg.ordered::<InputDock>()
        .into_iter()
        .map(|s| {
            (
                match s.key.as_str() {
                    "approval" => "approval_dock_title",
                    _ => "nav_queued",
                },
                s.order,
            )
        })
        .collect()
}

/// The digest-binding rule (ReviewArmour) at the plugin boundary: an approve
/// forwards the `content_digest` the operator was shown (drift 409s
/// server-side); a reject carries none. Pure so the plugin path is pinnable.
pub fn decision_digest(approve: bool, content_digest: &str) -> Option<String> {
    approve.then(|| content_digest.to_string())
}

#[component]
pub fn ApprovalDock() -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let roles = api().roles();
    let can_decide = role::role_allows(&roles, "approve");
    let connected = (ui.conn)() == Conn::Connected;

    let mut proposals = use_signal(Vec::<crate::api::Proposal>::new);
    let mut error = use_signal(|| None::<String>);

    // M2 composition: the dock renders only if it survives the SHARED plugin
    // host's fail-closed visibility pass (the control-panel plugin registered
    // this dock at order 5, role-gated; third parties could reorder or gate).
    let host = use_context::<Signal<crate::plugins::PluginHost>>();
    let caps: Vec<String> = {
        let mut c: Vec<String> = roles.iter().map(|r| format!("role:{r}")).collect();
        if can_decide {
            c.push("role:approve".to_string());
        }
        c
    };
    let docks_visible = {
        let reg = &host.read().slots;
        crate::ui_renderer::input_docks(reg, &caps)
            .iter()
            .any(|d| d.key == "approval")
    };

    use_resource(move || async move {
        if !connected {
            return;
        }
        match api().proposals("pending").await {
            Ok(list) => {
                proposals.set(list);
                error.set(None);
            }
            Err(_) => error.set(Some(t("dock_load_failed").to_string())),
        }
    });

    let decide = move |approve: bool, id: i64, digest: Option<String>| {
        let mut proposals = proposals;
        let mut error = error;
        let mut ui_pending = ui.pending_count;
        spawn(async move {
            let client = api();
            let res = if approve {
                // The plugin boundary never loosens the digest binding: the
                // approve carries exactly the rendered digest (drift 409s).
                let bound = decision_digest(true, &digest.unwrap_or_default());
                client
                    .approve_proposal(id, None, bound.as_deref())
                    .await
                    .map(|_| ())
            } else {
                client.reject_proposal(id, None).await.map(|_| ())
            };
            match res {
                Ok(()) => {
                    proposals.with_mut(|p| p.retain(|x| x.id != id));
                    // Keep the shell's pending anchor honest.
                    ui_pending.set(proposals().len().min(u32::MAX as usize) as u32);
                }
                Err(e) => {
                    // A 409 here is the digest-drift guard doing its job.
                    error.set(Some(crate::api::error_message(&e).to_string()));
                }
            }
        });
    };

    let now = time_budget::now_unix();

    rsx! {
        if docks_visible {
            section {
                class: "card card-enhanced mb-4",
                "aria-label": t("approval_dock_title"),
                div { class: "card-header flex items-center justify-between",
                    h2 { class: "card-title text-base", {t("approval_dock_title")} }
                    span { class: "badge", role: "status",
                        "{proposals().len()} {t(\"pending_suffix\")}"
                    }
                }
                div { class: "card-body space-y-2",
                if let Some(e) = error() {
                    p { class: "text-sm text-danger shake", role: "alert", "{e}" }
                }
                if proposals().is_empty() && error().is_none() {
                    p { class: "text-sm text-muted-foreground", {t("dock_empty")} }
                }
                for (p, clean_content, invisible_removed) in proposals().iter().map(|p| {
                    let (clean, removed) = crate::strip_invisible_counted(&p.content);
                    (p.clone(), clean, removed)
                }) {
                    div {
                        class: "rounded-lg border border-border p-3 hover-lift will-animate",
                        div { class: "flex items-start justify-between gap-3",
                            // The full content, in a bounded scroll box — a
                            // two-line clamp is truncation-evasion by default.
                            div { class: "max-h-40 overflow-y-auto whitespace-pre-wrap flex-1 rounded border border-border/50 p-2 text-sm text-foreground",
                                "{clean_content}"
                            }
                            span { class: "badge tabular shrink-0", "#{p.id}" }
                        }
                        if invisible_removed > 0 {
                            p { class: "mt-1 badge badge-warn",
                                {t_fmt("dock_invisible_removed", &[invisible_removed.to_string()])}
                            }
                        }
                        div { class: "mt-1 text-xs text-ink-faint tabular",
                            {t_fmt(
                                "dock_sla",
                                &[time_budget::format_remaining(
                                    time_budget::remaining(p.expires_at, now),
                                )],
                            )}
                        }
                        if can_decide {
                            div { class: "mt-2 flex gap-2",
                                button {
                                    class: "btn btn-primary btn-sm",
                                    "aria-label": t_fmt("dock_approve_aria", &[p.id.to_string()]),
                                    onclick: move |_| decide(true, p.id, Some(p.content_digest.clone())),
                                    {t("approve")}
                                }
                                button {
                                    class: "btn btn-outline btn-sm",
                                    "aria-label": t_fmt("dock_reject_aria", &[p.id.to_string()]),
                                    onclick: move |_| decide(false, p.id, None),
                                    {t("reject")}
                                }
                            }
                        }
                    }
                }
            }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dock decides from FULL content in a bounded scroll box — never a
    /// two-line clamp (truncation-evasion by default). Source-pinned.
    #[test]
    fn dock_renders_full_content_not_a_clamp() {
        let src = include_str!("approvals.rs");
        assert!(
            src.contains("max-h-40 overflow-y-auto whitespace-pre-wrap"),
            "dock content must use the bounded scroll box pattern"
        );
        let clamp = concat!("line-clamp", "-2");
        assert!(!src.contains(clamp), "no clamp class may remain");
    }

    #[test]
    fn dock_order_is_approval_then_queue() {
        let orders = dock_orders();
        assert_eq!(orders.len(), 2);
        assert_eq!(orders[0].0, "approval_dock_title");
        assert_eq!(orders[0].1, 5);
        assert_eq!(orders[1].1, 20);
    }

    #[test]
    fn plugin_path_binds_digest_on_approve_only() {
        // Approve binds the bytes the operator saw (server 409s on drift).
        assert_eq!(
            decision_digest(true, "sha256-abc"),
            Some("sha256-abc".to_string())
        );
        // Reject is always safe and carries no digest.
        assert_eq!(decision_digest(false, "sha256-abc"), None);
    }
}
