//! Portability group — UMP panel (v1.17.8 M6). The operator surface for the
//! v1.17.3/v1.17.4 UMP 1.0 wire layer:
//! - capabilities handshake (`GET /ump/capabilities`) + integrity badge
//! - remember / revise / forget / feedback (write ops)
//! - recall (`POST /ump/recall`, `max_recall` honored)
//! - audit + chain verify (`POST /ump/audit`, `GET /ump/audit/verify`)
//!
//! Records are rendered as pretty JSON; the raw wire is shown, not a re-
//! interpretation, so the operator sees exactly what the reference suite sees.

use crate::api::ApiClient;
use crate::panels::{use_document_title, PageTitle};
use dioxus::prelude::*;

pub fn panel() -> Element {
    use_document_title(|| "UMP — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<crate::UiState>();
    let writes = (ui.writes_enabled)();

    let mut caps = use_signal(|| None::<crate::api::UmpCapabilities>);
    let mut status = use_signal(|| None::<Result<String, String>>);

    let mut remember_body = use_signal(String::new);
    let mut recall_q = use_signal(String::new);
    let mut recall_kind = use_signal(String::new);
    let audit_limit = use_signal(|| 50usize);

    let mut results = use_signal(Vec::<crate::api::UmpRecallResult>::new);
    let mut audit = use_signal(|| None::<crate::api::UmpAudit>);
    let mut verify = use_signal(|| None::<bool>);

    let load_caps = move |_| {
        let api = api;
        spawn(async move {
            match api().ump_capabilities().await {
                Ok(c) => caps.set(Some(c)),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    // Load capabilities once on mount, not on every render. An unconditional
    // `load_caps(())` in the body re-fires per keystroke (the panel re-renders
    // as `recall_q`/`status`/`results` change), stacking a `/ump/capabilities`
    // request per keystroke on top of the 5s `/health` probe — enough to trip
    // the server's per-IP 10k/60s limiter (then `/health` also 429s → the
    // client's "rate limited" + "reconnecting"). The ⟳ button keeps a manual
    // refresh.
    use_effect(move || load_caps(()));

    let mut run_remember = move |_| {
        let api = api;
        let body: serde_json::Value = match serde_json::from_str(&remember_body()) {
            Ok(v) => v,
            Err(e) => {
                status.set(Some(Err(format!(
                    "{}: {e}",
                    crate::i18n::t("ump_bad_json")
                ))));
                return;
            }
        };
        spawn(async move {
            match api().ump_remember(&body).await {
                Ok(r) => status.set(Some(Ok(format!(
                    "{} · {}",
                    crate::i18n::t("ump_remembered"),
                    r.id
                )))),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_recall = move |_| {
        let api = api;
        let q = recall_q().trim().to_string();
        let kind = recall_kind().trim().to_string();
        spawn(async move {
            let limit = caps().map(|c| c.max_recall.clamp(1, 100)).unwrap_or(50) as u32;
            match api().ump_recall(&q, limit, Some(&kind)).await {
                Ok(r) => results.set(r.results),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_audit = move |_| {
        let api = api;
        let limit = audit_limit();
        spawn(async move {
            match api().ump_audit(None, limit).await {
                Ok(a) => audit.set(Some(a)),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_verify = move |_| {
        let api = api;
        spawn(async move {
            match api().ump_audit_verify().await {
                Ok(r) => verify.set(Some(r.ok)),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let caps_lbl = crate::i18n::t("ump_caps");
    let remember_lbl = crate::i18n::t("ump_remember");
    let recall_lbl = crate::i18n::t("ump_recall");
    let audit_lbl = crate::i18n::t("ump_audit");
    let title_lbl = crate::i18n::t("ump_title");
    let sub_lbl = crate::i18n::t("ump_sub");
    let caps_opt = caps();
    let (srv_line, ump_ver_line, (integrity_badge, integrity_label), kinds_line) = match &caps_opt {
        Some(c) => (
            format!("{} {}", c.server.name, c.server.version),
            format!("UMP {}", c.ump),
            crate::api::ump_integrity_badge(&c.conformance),
            c.kinds.join(" "),
        ),
        None => (
            String::new(),
            String::new(),
            ("", String::new()),
            String::new(),
        ),
    };

    rsx! {
        PageTitle { "{title_lbl}" }
        p { class: "text-sm text-muted-foreground mb-4", "{sub_lbl}" }

        div { class: "card",
            div { class: "card-header",
                div { class: "card-title", "{caps_lbl}" }
                button { class: "btn btn-outline btn-sm", onclick: move |_| load_caps(()), "⟳" }
            }
            div { class: "card-body text-sm",
                if let Some(c) = caps_opt {
                    div { class: "flex flex-wrap items-center gap-4",
                        span { class: "font-mono text-xs", "{srv_line}" }
                        span { class: "font-mono text-xs", "{ump_ver_line}" }
                        span { class: "badge {integrity_badge}", "{integrity_label}" }
                        if c.writable { span { class: "badge badge-ok", "writable" } }
                        if c.audit { span { class: "badge badge-info", "audit" } }
                    }
                    if !c.kinds.is_empty() {
                        div { class: "mt-2 text-xs text-muted-foreground",
                            "{kinds_line}"
                        }
                    }
                } else {
                    p { class: "text-muted-foreground", "…" }
                }
            }
        }

        div { class: "card",
            div { class: "card-header", div { class: "card-title", "{remember_lbl}" } }
            div { class: "card-body space-y-2",
                textarea {
                    class: "input font-mono text-xs",
                    rows: 5,
                    value: "{remember_body}",
                    oninput: move |e| remember_body.set(e.value()),
                    placeholder: "content...",
                }
                button { class: "btn btn-primary", disabled: !writes, onclick: move |_| run_remember(()), "{remember_lbl}" }
            }
        }

        div { class: "card",
            div { class: "card-header", div { class: "card-title", "{recall_lbl}" } }
            div { class: "card-body space-y-2",
                div { class: "flex items-center gap-2",
                    input {
                        class: "input flex-1",
                        value: "{recall_q}",
                        oninput: move |e| recall_q.set(e.value()),
                        onkeydown: move |e| if e.key() == Key::Enter { run_recall(()) },
                        placeholder: "query…",
                    }
                    input {
                        class: "input w-32",
                        value: "{recall_kind}",
                        oninput: move |e| recall_kind.set(e.value()),
                        placeholder: "kind (opt)",
                    }
                    button { class: "btn btn-outline", onclick: move |_| run_recall(()), "{recall_lbl}" }
                }
                if !results().is_empty() {
                    ul { class: "space-y-1",
                        for r in results().iter() {
                            li { class: "rounded border border-border p-2 text-xs",
                                div { class: "font-mono", "{serde_json::to_string_pretty(&r.record).unwrap_or_default()}" }
                                div { class: "text-muted-foreground mt-1", "score {r.score}" }
                            }
                        }
                    }
                }
            }
        }

        div { class: "card",
            div { class: "card-header",
                div { class: "card-title", "{audit_lbl}" }
                div { class: "flex items-center gap-2",
                    button { class: "btn btn-outline btn-sm", onclick: move |_| run_audit(()), "load" }
                    button { class: "btn btn-outline btn-sm", onclick: move |_| run_verify(()), "verify chain" }
                }
            }
            div { class: "card-body text-xs",
                if let Some(ok) = verify() {
                    if ok {
                        span { class: "text-ok", {crate::i18n::t("ump_chain_ok")} }
                    } else {
                        span { class: "text-danger", {crate::i18n::t("ump_chain_bad")} }
                    }
                }
                if let Some(a) = audit() {
                    p { class: "text-muted-foreground", "{a.count} rows" }
                    pre { class: "overflow-auto rounded border border-border p-2",
                        {serde_json::to_string_pretty(&serde_json::json!({"rows": &a.rows})).unwrap_or_default()}
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
