//! System group — Console panel (v1.17.8 M7). The operator admin + try-it
//! surface, stacked as cards on one page:
//! - domains (`GET /domains`), snapshot integrity (`GET /snapshot/status`),
//!   Art 30 register (`GET /art30`), and reindex (`POST /reindex`)
//! - sources reconcile (`POST /sources/reconcile`) + connectors
//!   (`GET /connectors`)
//! - a try-it console that sends arbitrary requests through the same
//!   `ApiClient` and logs redacted lines (M7.3).
//!
//! One page, one nav target; each card is self-contained (the create.rs
//! pattern).

use crate::api::ApiClient;
use crate::confirm::ConfirmDestructive;
use crate::panels::{PageTitle, use_document_title};
use dioxus::prelude::*;

/// M3.1: a boolean evidence cell renders via an i18n key (yes/no), never the
/// raw English `true`/`false` from JSON. Pure so the scan guard cannot trip.
fn sys_bool(v: bool) -> &'static str {
    if v { "sys_yes" } else { "sys_no" }
}

pub fn panel() -> Element {
    use_document_title(|| format!("{} — brain", crate::i18n::t("sys_title")));
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<crate::UiState>();
    let writes = (ui.writes_enabled)();

    let mut domains = use_signal(Vec::<crate::api::DomainInfo>::new);
    let mut snapshot = use_signal(|| None::<crate::api::SnapshotStatus>);
    let mut art30 = use_signal(|| None::<serde_json::Value>);
    let mut connectors = use_signal(Vec::<crate::api::ConnectorRow>::new);
    let mut reindex_r = use_signal(|| None::<crate::api::ReindexResult>);
    let mut reconcile_r = use_signal(|| None::<crate::api::ReconcileResult>);
    let mut status = use_signal(|| None::<Result<String, String>>);

    // Console (M7.3)
    let mut method = use_signal(|| "GET".to_string());
    let mut path = use_signal(String::new);
    let mut body = use_signal(String::new);
    let mut history = use_signal(Vec::<crate::api::StoredLine>::new);

    // M1 (v1.18.1): the console's non-secret history survives reload. Only
    // `redact_for_history`-clean lines persist (via the existing i18n pref
    // seam); `secret` lines stay in-memory. Raw token-bearing input is never
    // written — the `credentials_stay_in_memory` grep guard still holds.
    const HISTORY_CAP: usize = 100;
    let sys_multi_domains = crate::i18n::t("sys_multi_domains");
    let sys_http_get = crate::i18n::t("sys_http_get");
    let sys_http_post = crate::i18n::t("sys_http_post");
    let sys_http_delete = crate::i18n::t("sys_http_delete");
    use_effect(move || {
        spawn(async move {
            if let Some(saved) = crate::i18n::pref_load("console_history").await
                && history().is_empty()
            {
                let lines = saved
                    .split('\n')
                    .filter(|s| !s.is_empty())
                    .map(|s| crate::api::StoredLine {
                        text: s.to_string(),
                        secret: false,
                    })
                    .collect();
                history.set(lines);
            }
        });
    });

    let load = move |_| {
        let api = api;
        spawn(async move {
            match api().domains().await {
                Ok(d) => domains.set(d.domains),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
            // v1.27.19 "Scrub" (D-7): was `if let Ok` — the panel silently
            // stayed empty-stale on failure; now each secondary load is loud.
            match api().snapshot_status().await {
                Ok(s) => snapshot.set(Some(s)),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
            match api().art30().await {
                Ok(a) => art30.set(Some(a)),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
            match api().connectors().await {
                Ok(c) => connectors.set(c.connectors),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };
    // v1.27.20 "Console" (F-40): load ONCE on mount, not on every render. An
    // unconditional `load(())` in the body re-fires per keystroke/re-render
    // (the panel re-renders as `status`/`reindex_r`/`history` change), stacking
    // a 4-request batch per keystroke. Same pattern the ump.rs panel uses for
    // its own mount-once fetch; the ⟳ button keeps a manual refresh.
    use_effect(move || load(()));

    let run_reindex = move |_| {
        let api = api;
        spawn(async move {
            match api().reindex().await {
                Ok(r) => {
                    reindex_r.set(Some(r.clone()));
                    status.set(Some(Ok(crate::i18n::t_fmt(
                        "sys_reindexed",
                        &[r.reembedded.to_string()],
                    ))));
                }
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_reconcile = move |_| {
        let api = api;
        spawn(async move {
            // v1.27.21 (N1): the panel reconciles with an EMPTY live set —
            // that retires EVERY vault source and sweeps its chunks, so the
            // button is a two-step confirm and the request waives the
            // server's `live_set_empty` guard explicitly (`allow_empty`).
            match api().sources_reconcile("vault", &[], true).await {
                Ok(r) => reconcile_r.set(Some(r)),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_console = move |_| {
        let api = api;
        let m = method().to_uppercase();
        let p = path().trim().to_string();
        let b = body().trim().to_string();
        let redacted = crate::api::redact_for_history(&b);
        let line = crate::api::serialize_request(&m, &p, &redacted);
        let entry = crate::api::StoredLine {
            text: line,
            secret: crate::api::line_is_secret(&b),
        };
        history.write().push(entry);
        // M1: persist only the clean subset; secret/empty lines never touch disk.
        let clean = crate::api::persist_history(history().clone(), HISTORY_CAP);
        crate::i18n::pref_save("console_history", &clean.join("\n"));
        spawn(async move {
            let resp = if m == "GET" {
                api().get_raw(&p).await
            } else if m == "DELETE" {
                api().delete_raw(&p).await
            } else {
                api().post_raw(&p, &b).await
            };
            // The raw response body or the human error — never silence.
            let rendered = resp
                .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
                .map_err(|e| crate::api::error_message(&e));
            status.set(Some(rendered));
        });
    };

    let domains_lbl = crate::i18n::t("sys_domains");
    let snapshot_lbl = crate::i18n::t("sys_snapshot");
    let art30_lbl = crate::i18n::t("sys_art30");
    let reindex_lbl = crate::i18n::t("sys_reindex");
    let sources_lbl = crate::i18n::t("sys_sources");
    let console_lbl = crate::i18n::t("sys_console");
    let title_lbl = crate::i18n::t("sys_title");
    let sys_col_file = crate::i18n::t("sys_col_file");
    let sys_col_size = crate::i18n::t("sys_col_size");
    let sys_col_perms = crate::i18n::t("sys_col_perms");
    let sys_col_integrity = crate::i18n::t("sys_col_integrity");
    let sys_col_chain = crate::i18n::t("sys_col_chain");
    let sub_lbl = crate::i18n::t("sys_sub");

    rsx! {
        PageTitle { "{title_lbl}" }
        p { class: "text-sm text-muted-foreground mb-4", "{sub_lbl}" }

        div { class: "card",
            div { class: "card-header",
                div { class: "card-title", "{domains_lbl}" }
                button { class: "btn btn-outline btn-sm", onclick: move |_| load(()), "⟳" }
            }
            div { class: "card-body",
                if domains().is_empty() {
                    p { class: "text-sm text-muted-foreground", "…" }
                } else {
                    ul { class: "space-y-1",
                        for d in domains().iter() {
                            li { class: "flex items-center justify-between rounded border border-border p-2 text-sm",
                                span { class: "font-mono text-xs", "{d.name}" }
                                span { class: "text-muted-foreground text-xs",
                                    "{d.entries} · {d.entities} e · {d.relations} r"
                                    {if d.multi_db { sys_multi_domains.as_str() } else { "" }}
                                }
                            }
                        }
                    }
                }
            }
        }

        div { class: "card",
            div { class: "card-header", div { class: "card-title", "{snapshot_lbl}" } }
            div { class: "card-body text-sm",
                if let Some(s) = snapshot() {
                    div { class: "flex items-center gap-2",
                        span { class: if s.all_ok { "badge badge-ok" } else { "badge badge-danger" },  // i18n-exempt: css class expression
                            {crate::i18n::t(if s.all_ok {"sys_snapshot_ok"} else {"sys_snapshot_degraded"})}
                        }
                        span { class: "text-muted-foreground", {crate::i18n::t_fmt("sys_snapshot_count", &[s.snapshot_count.to_string()])} }
                    }
                    if !s.snapshots.is_empty() {
                        table { class: "table mt-2",
                            thead { tr {
                                th { class: "text-start pe-2", "{sys_col_file}" }
                                th { class: "text-start pe-2", "{sys_col_size}" }
                                th { class: "text-start pe-2", "{sys_col_perms}" }
                                th { class: "text-start pe-2", "{sys_col_integrity}" }
                                th { class: "text-start pe-2", "{sys_col_chain}" }
                            } }
                            tbody {
                                for r in s.snapshots.iter() {
                                    tr {
                                        td { class: "pe-2 font-mono text-xs", "{r.file}" }
                                        td { class: "pe-2 tabular text-xs", "{r.size_bytes}" }
                                        td { class: "pe-2",
                                            span { class: if r.mode_0600 { "badge badge-ok" } else { "badge badge-danger" },  // i18n-exempt: css class expression
                                                {crate::i18n::t(if r.mode_0600 {"sys_perms_0600"} else {"sys_world_readable"})}
                                            }
                                        }
                                        td { class: "pe-2",
                                            span {
                                                class: if r.integrity_check { "text-ok" } else { "text-danger" },  // i18n-exempt: css class expression
                                                {crate::i18n::t(sys_bool(r.integrity_check))}
                                            }
                                        }
                                        td { class: "pe-2",
                                            span {
                                                class: if r.audit_chain_ok { "text-ok" } else { "text-danger" },  // i18n-exempt: css class expression
                                                {crate::i18n::t(sys_bool(r.audit_chain_ok))}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    p { class: "text-muted-foreground", "…" }
                }
                div { class: "mt-3 flex items-center gap-2",
                    // v1.28.1 M4 (F-35): reindex rebuilds the vector store — the same
                    // two-step confirm the palette gives destructive runs
                    // (`destructive_action(RunAction::Reindex)`), so the raw
                    // snapshot-card button can no longer fire one-click.
                    ConfirmDestructive {
                        label: reindex_lbl.clone(),
                        note: crate::i18n::t("reindex_irreversible"),
                        small: false,
                        blocked: false,
                        disabled: !writes,
                        on_confirm: move |_| run_reindex(()),
                    }
                    if let Some(r) = reindex_r() {
                        span { class: "text-xs text-muted-foreground",
                            {crate::i18n::t_fmt("sys_reindex_result", &[r.status.clone(), r.reembedded.to_string(), r.skipped.to_string()])}
                        }
                    }
                }
            }
        }

        div { class: "card",
            div { class: "card-header", div { class: "card-title", "{art30_lbl}" } }
            div { class: "card-body",
                if let Some(a) = art30() {
                    pre { class: "overflow-auto rounded border border-border p-2 text-xs",
                        {serde_json::to_string_pretty(&a).unwrap_or_default()}
                    }
                } else {
                    p { class: "text-sm text-muted-foreground", "…" }
                }
            }
        }

        div { class: "card",
            div { class: "card-header", div { class: "card-title", "{sources_lbl}" } }
            div { class: "card-body space-y-2 text-sm",
                if connectors().is_empty() {
                    p { class: "text-muted-foreground", "…" }
                } else {
                    ul { class: "space-y-1",
                        for c in connectors().iter() {
                            li { class: "flex items-center justify-between rounded border border-border p-2 text-xs",
                                span { class: "font-mono", "{c.kind} · {c.instance}" }
                                span { class: "text-muted-foreground", "{c.state}" }
                            }
                        }
                    }
                }
                // v1.27.21 (N1): reconcile with an empty live set is a mass
                // retirement — the same two-step confirm as reindex/purge,
                // with the consequence spelled out before the second click.
                ConfirmDestructive {
                    label: crate::i18n::t("sys_reconcile"),
                    note: crate::i18n::t("sys_reconcile_irreversible"),
                    small: true,
                    blocked: false,
                    disabled: !writes,
                    on_confirm: move |_| run_reconcile(()),
                }
                if let Some(r) = reconcile_r() {
                    span { class: "text-xs text-muted-foreground",
                        {crate::i18n::t_fmt("sys_reconcile_result", &[r.deleted_sources.to_string(), r.deleted_chunks.to_string()])}
                    }
                }
            }
        }

        div { class: "card",
            div { class: "card-header", div { class: "card-title", "{console_lbl}" } }
            div { class: "card-body space-y-2",
                div { class: "flex items-center gap-2",
                    select {
                        class: "select w-28",
                        value: "{method}",
                        onchange: move |e| method.set(e.value()),
                        "aria-label": crate::i18n::t("aria_method"),
                        option { value: "GET", "{sys_http_get}" }
                        option { value: "POST", "{sys_http_post}" }
                        option { value: "DELETE", "{sys_http_delete}" }
                    }
                    input {
                        class: "input flex-1 font-mono",
                        value: "{path}",
                        oninput: move |e| path.set(e.value()),
                        placeholder: crate::i18n::t("sys_path_ph"),
                    }
                    button { class: "btn btn-primary", disabled: !writes, onclick: run_console, "send" }
                }
                if method() != "GET" && method() != "DELETE" {
                    textarea {
                        class: "input font-mono text-xs",
                        rows: 4,
                        value: "{body}",
                        oninput: move |e| body.set(e.value()),
                        placeholder: crate::i18n::t("sys_body_ph"),
                    }
                }
                if !history().is_empty() {
                    ul { class: "space-y-1",
                        for h in history().iter() {
                            li { class: "rounded border border-border p-2 text-xs font-mono",
                                "{h.text}"
                            }
                        }
                    }
                }
            }
        }

        div { "role": "status", "aria-live": "polite", class: "text-sm",
            match status() {
                Some(Ok(m)) => rsx! { span { class: "text-ok", "{m}" } },
                Some(Err(m)) => rsx! { span { class: "text-danger", "{m}" } },
                None => rsx! { span { class: "text-muted-foreground", "…" } },
            }
        }
    }
}

// ---------------------------------------------------------------------------
// v1.28.1 M4 (F-35) tests — the reindex confirm matches the palette gate.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    /// Reindex is destructive on EVERY surface: the command palette's
    /// `destructive_action` predicate (main.rs) AND the snapshots card's
    /// two-step confirm share the invariant that a single click cannot fire
    /// the rebuild. The panel button routes through the shared component;
    /// the palette keeps its own keyboard gate. Both answer the same question
    /// from the same pure core.
    #[test]
    fn reindex_button_confirms_like_palette() {
        // Palette parity: destructive_action(Reindex) is the selection gate.
        assert!(crate::destructive_action(&crate::RunAction::Reindex));
        // Component parity: arm ≠ fire, and the writes gate freezes both.
        assert!(crate::confirm::arm_allowed(false, false, false));
        assert!(!crate::confirm::confirm_allowed(false, false, false));
        assert!(crate::confirm::confirm_allowed(true, false, false));
        assert!(!crate::confirm::arm_allowed(false, true, false));
    }

    /// v1.27.21 (N1): reconcile with an empty live set retires EVERY vault
    /// source — a single click must never fire it. The panel routes the
    /// button through the shared two-step confirm, and the request carries
    /// the `allow_empty` waiver ONLY on that confirmed path (the wire's
    /// default omits it, so a stray caller meets the server's 400
    /// `live_set_empty` guard instead of a mass retirement).
    #[test]
    fn reconcile_confirms_and_waives_empty_live_set_only_on_confirm() {
        assert!(crate::confirm::arm_allowed(false, false, false));
        assert!(!crate::confirm::confirm_allowed(false, false, false));
        assert!(crate::confirm::confirm_allowed(true, false, false));
        assert!(!crate::confirm::arm_allowed(false, false, true));
        // The panel fires reconcile only through the confirm's on_confirm…
        let src = std::fs::read_to_string("src/panels/system.rs").unwrap();
        assert!(
            src.contains("label: crate::i18n::t(\"sys_reconcile\")"),
            "the reconcile button must be the shared two-step confirm"
        );
        // …and only the confirmed path waives the empty-live-set guard. The
        // needle is assembled at runtime so the counting line below does not
        // match itself (a contiguous literal would count as a second call).
        let needle = format!("{}{}{}", "api().", "sources_reconcile", "(");
        assert_eq!(src.matches(&needle).count(), 1);
        assert!(src.contains("sources_reconcile(\"vault\", &[], true)"));
        let guarded = crate::api::sources_reconcile_body("vault", &[], false);
        assert!(
            guarded.get("allow_empty").is_none(),
            "an unwaived body must let the server 400 live_set_empty"
        );
    }

    /// F-40 (v1.27.20 "Console"): the panel's 4-request batch load fires ONCE
    /// per mount, not on every render. Without a render harness, the honest
    /// pin is a source guard: the loader must be invoked through `use_effect`
    /// and never as a bare body statement (the pre-fix shape that stacked a
    /// `/domains`+`/snapshot/status`+`/art30`+`/connectors` batch per
    /// keystroke). The same grep-guard style as
    /// `interactive_elements_are_buttons` in main.rs.
    #[test]
    fn system_panel_fetches_once_per_mount() {
        let src = std::fs::read_to_string("src/panels/system.rs").unwrap();
        let lines: Vec<&str> = src.lines().collect();
        let body = lines[17..350].join("\n"); // panel() only, not the tests
        assert!(
            body.contains("use_effect(move || load(()))"),
            "the batch loader must be wired through use_effect (mount-once)"
        );
        // No bare body-statement call of `load(())` may exist outside the
        // use_effect line (the rm-fix regression this guard catches).
        for (i, line) in body.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//")
                || trimmed.starts_with("use_effect(")
                || trimmed.contains("use_effect")
            {
                continue;
            }
            assert!(
                !trimmed.starts_with("load(())"),
                "bare body call of load at {} — re-fires per render",
                i + 18,
            );
        }
    }
}
