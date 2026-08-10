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
use crate::panels::{use_document_title, PageTitle};
use dioxus::prelude::*;

pub fn panel() -> Element {
    use_document_title(|| "Console — brain".into());
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
    use_effect(move || {
        spawn(async move {
            if let Some(saved) = crate::i18n::pref_load("console_history").await {
                if history().is_empty() {
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
            if let Ok(s) = api().snapshot_status().await {
                snapshot.set(Some(s));
            }
            if let Ok(a) = api().art30().await {
                art30.set(Some(a));
            }
            if let Ok(c) = api().connectors().await {
                connectors.set(c.connectors);
            }
        });
    };
    load(());

    let run_reindex = move |_| {
        let api = api;
        spawn(async move {
            match api().reindex().await {
                Ok(r) => {
                    reindex_r.set(Some(r.clone()));
                    status
                        .set(Some(Ok(crate::i18n::t("sys_reindexed")
                            .replace("{n}", &r.reembedded.to_string()))));
                }
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_reconcile = move |_| {
        let api = api;
        spawn(async move {
            // Reconcile with an empty live set retires nothing; the operator
            // drives the URI set via the CLI. Surface the ledger counts here.
            match api().sources_reconcile("vault", &[]).await {
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
                api().post_raw(&p, b.clone()).await
            };
            match resp {
                Ok(v) => status.set(Some(Ok(
                    serde_json::to_string_pretty(&v).unwrap_or_default()
                ))),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let domains_lbl = crate::i18n::t("sys_domains");
    let snapshot_lbl = crate::i18n::t("sys_snapshot");
    let art30_lbl = crate::i18n::t("sys_art30");
    let reindex_lbl = crate::i18n::t("sys_reindex");
    let sources_lbl = crate::i18n::t("sys_sources");
    let console_lbl = crate::i18n::t("sys_console");
    let title_lbl = crate::i18n::t("sys_title");
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
                                    {if d.multi_db { " · multi" } else { "" }}
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
                        span { class: if s.all_ok { "badge badge-ok" } else { "badge badge-danger" },
                            {if s.all_ok {"ok"} else {"degraded"}}
                        }
                        span { class: "text-muted-foreground", "{s.snapshot_count} snapshots" }
                    }
                    if !s.snapshots.is_empty() {
                        table { class: "table mt-2",
                            thead { tr {
                                th { class: "text-left pr-2", "file" }
                                th { class: "text-left pr-2", "size" }
                                th { class: "text-left pr-2", "perms" }
                                th { class: "text-left pr-2", "integrity" }
                                th { class: "text-left pr-2", "audit chain" }
                            } }
                            tbody {
                                for r in s.snapshots.iter() {
                                    tr {
                                        td { class: "pr-2 font-mono text-xs", "{r.file}" }
                                        td { class: "pr-2 tabular text-xs", "{r.size_bytes}" }
                                        td { class: "pr-2",
                                            span { class: if r.mode_0600 { "badge badge-ok" } else { "badge badge-danger" },
                                                {if r.mode_0600 {"0600"} else {"world-readable"}}
                                            }
                                        }
                                        td { class: "pr-2",
                                            span { class: if r.integrity_check { "text-ok" } else { "text-danger" }, "{r.integrity_check}" }
                                        }
                                        td { class: "pr-2",
                                            span { class: if r.audit_chain_ok { "text-ok" } else { "text-danger" }, "{r.audit_chain_ok}" }
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
                    button {
                        class: "btn btn-primary",
                        disabled: !writes,
                        onclick: run_reindex,
                        "{reindex_lbl}"
                    }
                    if let Some(r) = reindex_r() {
                        span { class: "text-xs text-muted-foreground",
                            "{r.status} · {r.reembedded} re-embedded · {r.skipped} skipped"
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
                button {
                    class: "btn btn-outline btn-sm",
                    disabled: !writes,
                    onclick: run_reconcile,
                    "reconcile sources"
                }
                if let Some(r) = reconcile_r() {
                    span { class: "text-xs text-muted-foreground",
                        "{r.deleted_sources} retired · {r.deleted_chunks} chunks"
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
                        "aria-label": "method",
                        option { value: "GET", "GET" }
                        option { value: "POST", "POST" }
                        option { value: "DELETE", "DELETE" }
                    }
                    input {
                        class: "input flex-1 font-mono",
                        value: "{path}",
                        oninput: move |e| path.set(e.value()),
                        placeholder: "/recall",
                    }
                    button { class: "btn btn-primary", disabled: !writes, onclick: run_console, "send" }
                }
                if method() != "GET" && method() != "DELETE" {
                    textarea {
                        class: "input font-mono text-xs",
                        rows: 4,
                        value: "{body}",
                        oninput: move |e| body.set(e.value()),
                        placeholder: "query...",
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
