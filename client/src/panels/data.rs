//! Rights group — Data panel (v1.17.8 M5). The GDPR/portability surface:
//! - purge chunks by id or owner (`POST /purge`, Admin)
//! - portable export + download (`GET /export?format=json|ump|ump-md`)
//! - per-kind retention editor (`GET/POST /retention`, Admin)
//! - decayed list + tombstones (read-only registries, surfaced here alongside
//!   the purge + retention so the whole "rights" story lives in one place).
//!
//! Every mutation is operator-driven; nothing decays or deletes autonomously.

use crate::api::ApiClient;
use crate::confirm::ConfirmDestructive;
use crate::panels::{PageTitle, use_document_title};
use crate::time_budget::{Tier, format_remaining, now_unix, remaining, tier};
use dioxus::prelude::*;

/// v1.20.22 M2.2: day-scale next-expiry bands, the same numbers the Subjects
/// Art 17 clock uses (<3d warn, <1d danger).
const EXPIRY_WARN_SECS: i64 = 3 * 86400;
const EXPIRY_CRITICAL_SECS: i64 = 86400;

/// v1.20.22 M2.2: the "what expires next" pure core — sort decayed rows by
/// expiry ascending, keep the not-yet-expired ones, cap at 10. Returns
/// `(id, expiry_ts)`. ponytail: the real `/decayed` endpoint already filters
/// to expired rows (`page_decayed`, v1.20.18), so the empty-result case is the
/// honest current reality; the client boundary still exists in case the server
/// ever returns a near-expiry row.
pub fn next_expiries(decayed: &[crate::api::DecayedRow], now: i64) -> Vec<(i64, i64)> {
    let mut v: Vec<(i64, i64)> = decayed
        .iter()
        .filter_map(|d| d.effective_expiry.or(d.expires_at).map(|e| (d.id, e)))
        .filter(|(_, e)| *e > now)
        .collect();
    v.sort_by_key(|(_, e)| *e);
    v.truncate(10);
    v
}

pub fn panel() -> Element {
    use_document_title(|| "Data — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<crate::UiState>();
    let writes = (ui.writes_enabled)();

    // Purge
    let mut purge_ids = use_signal(String::new);
    let mut purge_owner = use_signal(String::new);
    let mut status = use_signal(|| None::<Result<String, String>>);
    // v1.28.1 M4 (F-12): the inline footprint-preview gate. The purge button
    // stays inert until the operator has RENDERED the preview card for the
    // CURRENT input — seeing and erasing are separated, and an edit invalidates
    // the preview (the snapshot must still equal the live input).
    let mut purge_preview = use_signal(|| None::<(String, String)>);

    let run_purge_preview = move |_| {
        purge_preview.set(Some((purge_ids(), purge_owner())));
    };

    // Retention
    let mut retention = use_signal(|| None::<crate::api::RetentionStatus>);
    let mut new_kind = use_signal(String::new);
    let mut new_days = use_signal(String::new);

    // Registries
    let mut decayed = use_signal(Vec::<crate::api::DecayedRow>::new);
    let mut tombstones = use_signal(Vec::<crate::api::TombstoneRow>::new);
    let mut loaded = use_signal(|| false);
    let mut loading = use_signal(|| false);

    let mut load = move |_| {
        let api = api;
        loading.set(true);
        spawn(async move {
            let r = api().retention().await;
            let d = api().decayed().await;
            let t = api().tombstones(100).await;
            match r {
                Ok(r) => retention.set(Some(r)),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
            // v1.27.19 "Scrub" (D-7): was `if let Ok` — a failed decayed/
            // tombstones load silently left the registries stale-empty.
            match d {
                Ok(d) => decayed.set(d),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
            match t {
                Ok(t) => tombstones.set(t.tombstones),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
            loading.set(false);
            loaded.set(true);
        });
    };

    let run_purge = move |_| {
        let api = api;
        let ids: Vec<i64> = purge_ids()
            .split([' ', ',', '\n'])
            .filter(|s| !s.trim().is_empty())
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        let owner = purge_owner();
        spawn(async move {
            if ids.is_empty() && owner.trim().is_empty() {
                status.set(Some(Err(crate::i18n::t("data_purge_empty"))));
                return;
            }
            match api().purge(&ids, Some(&owner)).await {
                Ok(r) => {
                    status.set(Some(Ok(crate::i18n::t_fmt(
                        "data_purged",
                        &[r.purged.to_string()],
                    ))));
                    load(());
                }
                Err(e) if crate::queue::is_offline(&e) => {
                    // v1.20.0 M3: unreachable backend → queue the purge for
                    // replay instead of failing the operator's request.
                    // v1.27.21 (N8): the owner travels WITH the ids — an
                    // owner-scoped purge replayed without its owner sends an
                    // empty body (a silent no-op dressed as an erasure).
                    let owner_opt = {
                        let o = owner.trim();
                        (!o.is_empty()).then(|| o.to_string())
                    };
                    crate::queue::enqueue(crate::queue::QueuedAction::Purge {
                        chunk_ids: ids.clone(),
                        owner: owner_opt,
                        queued_at: crate::queue::now_ts(),
                        retries: 0,
                    });
                    status.set(Some(Ok(crate::i18n::t("data_purged_queued"))));
                }
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_download = move |format: String| {
        let api = api;
        spawn(async move {
            match api().export(&format).await {
                Ok(body) => {
                    let name = match format.as_str() {
                        "ump" => "brain-export.ump.json",
                        "ump-md" => "brain-export.ump.md",
                        _ => "brain-export.json",
                    };
                    let s = body.to_string();
                    let js = format!(
                        "(function(){{var b=new Blob([{s:?}],{{type:'application/json'}});var u=URL.createObjectURL(b);var a=document.createElement('a');a.href=u;a.download='{name}';a.click();URL.revokeObjectURL(u);}})();"
                    );
                    let _ = document::eval(&js);
                    status.set(Some(Ok(crate::i18n::t("data_exported"))));
                }
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_retention = move |_| {
        let api = api;
        let kind = new_kind().trim().to_string();
        let days: i64 = match new_days().trim().parse() {
            Ok(d) => d,
            Err(_) => {
                status.set(Some(Err(crate::i18n::t("data_retention_bad_days"))));
                return;
            }
        };
        spawn(async move {
            match api().retention_set(&kind, days).await {
                Ok(r) => {
                    status.set(Some(Ok(crate::i18n::t_fmt(
                        "data_retention_set",
                        &[r.updated.to_string()],
                    ))));
                    load(());
                }
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let run_retention_clear = move |kind: String| {
        let api = api;
        spawn(async move {
            match api().retention_clear(&kind).await {
                Ok(_) => load(()),
                Err(e) => status.set(Some(Err(crate::api::error_message(&e)))),
            }
        });
    };

    let status_lbl = crate::i18n::t("data_status");
    let purged_lbl = crate::i18n::t("data_purge");
    let export_lbl = crate::i18n::t("data_export");
    let ret_lbl = crate::i18n::t("data_retention");
    let decayed_lbl = crate::i18n::t("data_decayed");
    let tombs_lbl = crate::i18n::t("data_tombstones");
    let empty_lbl = crate::i18n::t("data_empty");
    let ret_state_lbl = crate::i18n::t("data_retention_state");
    let ret_kind_lbl = crate::i18n::t("data_retention_kind");
    let ret_days_lbl = crate::i18n::t("data_retention_days");

    // Hoisted reads so the rsx stays statement-free (a `let` as a direct child
    // of an `if let` breaks the rsx parser).
    let retention_opt = retention();
    let ret_edits = retention_opt
        .as_ref()
        .map(|r| crate::api::retention_to_edits(&r.policy))
        .unwrap_or_default();
    let ret_state_line = retention_opt.as_ref().map(|r| {
        let state = if r.enabled { "enabled" } else { "disabled" };
        format!("{ret_state_lbl}: {state} · {}", r.projection)
    });
    let decayed_rows: Vec<_> = decayed();
    let next = next_expiries(&decayed_rows, now_unix());
    // Precompute (id, class, label) so the rsx for-loop body stays a pure list
    // of elements (a `let` verbatim in a for-body breaks the rsx parser).
    let next_views: Vec<(i64, &'static str, String)> = next
        .iter()
        .map(|(id, expires_at)| {
            let now = now_unix();
            let t = tier(
                remaining(*expires_at, now),
                EXPIRY_WARN_SECS,
                EXPIRY_CRITICAL_SECS,
            );
            let cls = match t {
                Tier::Critical | Tier::Expired => "badge badge-danger",
                Tier::Warn => "badge badge-warn",
                Tier::Ok => "badge badge-ok",
            };
            (*id, cls, format_remaining(remaining(*expires_at, now)))
        })
        .collect();
    let tomb_rows: Vec<(i64, String)> = tombstones()
        .iter()
        .map(|t| (t.knowledge_id, t.reason.clone().unwrap_or_default()))
        .collect();

    // v1.28.1 M4 (F-12): purge-preview gate locals — computed once per render.
    // `preview_fresh` = a preview card has been rendered for the CURRENT ids +
    // owner input; the purge confirm stays frozen until it holds.
    let purge_snap = purge_preview();
    let purge_preview_ids = purge_snap
        .as_ref()
        .map(|(ids, _)| crate::panels::data::parse_purge_ids(ids).len())
        .unwrap_or(0);
    let preview_fresh = purge_snap
        .as_ref()
        .map(|(ids, owner)| {
            crate::panels::data::purge_preview_fresh(&purge_ids(), &purge_owner(), ids, owner)
        })
        .unwrap_or(false);

    let data_ids_lbl = crate::i18n::t("data_ids_lbl");
    let data_owner_lbl = crate::i18n::t("data_owner_lbl");
    let data_json = crate::i18n::t("data_json");
    let data_ump = crate::i18n::t("data_ump");
    let data_ump_md = crate::i18n::t("data_ump_md");

    rsx! {
        PageTitle { {crate::i18n::t("data_title")} }
        p { class: "text-sm text-muted-foreground mb-4", {crate::i18n::t("data_sub")} }

        div { class: "card",
            div { class: "card-header",
                div { class: "card-title", "{purged_lbl}" }
                button { class: "btn btn-outline btn-sm", onclick: move |_| load(()), "⟳" }
            }
            div { class: "card-body space-y-3",
                label { class: "label", {crate::i18n::t("data_purge_ids")} }
                textarea {
                    class: "input",
                    rows: 2,
                    value: "{purge_ids}",
                    oninput: move |e| purge_ids.set(e.value()),
                    placeholder: crate::i18n::t("data_ids_ph"),
                }
                label { class: "label", {crate::i18n::t("data_purge_owner")} }
                input {
                    class: "input",
                    value: "{purge_owner}",
                    oninput: move |e| purge_owner.set(e.value()),
                    placeholder: crate::i18n::t("data_owner_ph"),
                }
                // v1.28.1 M4 (F-12): the inline footprint preview — rendered
                // BEFORE the purge can arm. The card snapshots the input at
                // preview time; an edit after the preview leaves the snapshot
                // stale, so the purge button stays frozen (`preview_fresh`).
                button {
                    class: "btn btn-outline btn-sm",
                    disabled: !writes,
                    onclick: run_purge_preview,
                    {crate::i18n::t("data_purge_preview")}
                }
                if purge_snap.is_some() {
                    div { class: "card border-dashed",
                        div { class: "card-body space-y-1",
                            p { class: "text-sm text-muted-foreground",
                                {crate::i18n::t("data_purge_preview_note")} }
                            dl { class: "grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm",
                                dt { class: "text-muted-foreground", "{data_ids_lbl}" }
                                dd { class: "font-mono tabular", "{purge_preview_ids}" }
                                dt { class: "text-muted-foreground", "{data_owner_lbl}" }
                                dd { class: "font-mono tabular",
                                    "{purge_snap.as_ref().unwrap().1}" }
                            }
                            if !preview_fresh {
                                p { class: "text-xs text-warn", {crate::i18n::t("data_purge_preview_stale")} }
                            }
                        }
                    }
                }
                div { class: "flex items-center gap-3",
                    ConfirmDestructive {
                        label: purged_lbl.clone(),
                        note: crate::i18n::t("purge_irreversible"),
                        small: false,
                        blocked: !preview_fresh,
                        blocked_hint: Some(crate::i18n::t("data_purge_need_preview")),
                        disabled: !writes,
                        on_confirm: run_purge,
                    }
                    if purge_preview().is_none() {
                        span { class: "text-xs text-ink-faint", {crate::i18n::t("data_purge_hint")} }
                    }
                }
            }
        }

        div { class: "card",
            div { class: "card-header", div { class: "card-title", "{export_lbl}" } }
            div { class: "card-body flex items-center gap-3",
                button { class: "btn btn-outline", onclick: move |_| run_download("json".into()), "{data_json}" }
                button { class: "btn btn-outline", onclick: move |_| run_download("ump".into()), "{data_ump}" }
                button { class: "btn btn-outline", onclick: move |_| run_download("ump-md".into()), "{data_ump_md}" }
            }
        }

        div { class: "card",
            div { class: "card-header", div { class: "card-title", "{ret_lbl}" } }
            div { class: "card-body space-y-3",
                if let Some(state_line) = ret_state_line {
                    div { class: "text-sm text-muted-foreground", "{state_line}" }
                    if ret_edits.is_empty() {
                        p { class: "text-sm text-muted-foreground", "{empty_lbl}" }
                    } else {
                        ul { class: "space-y-1",
                            for (kind, days) in ret_edits.clone() {
                                li { class: "flex items-center justify-between rounded border border-border p-2 text-sm",
                                    span { class: "font-mono text-xs", "{kind}" }
                                    span { class: "flex items-center gap-2",
                                        span { class: "text-muted-foreground", "{days}d" }
                                        button {
                                            class: "btn btn-ghost btn-sm",
                                            disabled: !writes,
                                            onclick: move |_| run_retention_clear(kind.clone()),
                                            "×"
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if loading() {
                    p { class: "text-muted-foreground text-sm", "…" }
                }
                div { class: "mt-3 flex items-end gap-2",
                    div { class: "flex-1",
                        label { class: "label", "{ret_kind_lbl}" }
                        input { class: "input", value: "{new_kind}", oninput: move |e| new_kind.set(e.value()), placeholder: crate::i18n::t("data_kind_ph") }
                    }
                    div { class: "w-28",
                        label { class: "label", "{ret_days_lbl}" }
                        input { class: "input", value: "{new_days}", oninput: move |e| new_days.set(e.value()), placeholder: crate::i18n::t("data_days_ph") }
                    }
                    button { class: "btn btn-primary", disabled: !writes, onclick: run_retention, "set" }
                }
            }
        }

        div { class: "card",
            div { class: "card-header",
                div { class: "card-title", "{decayed_lbl}" }
                span { class: "text-sm text-muted-foreground", "{decayed_rows.len()}" }
            }
            div { class: "card-body",
                if decayed_rows.is_empty() {
                    p { class: "text-sm text-muted-foreground", "{empty_lbl}" }
                } else {
                    ul { class: "space-y-1",
                        for d in decayed_rows {
                            li { class: "flex items-center justify-between rounded border border-border p-2 text-sm",
                                span { class: "font-mono text-xs", "#{d.id}" }
                                span { class: "text-muted-foreground text-xs", "{d.memory_kind}" }
                            }
                        }
                    }
                }
            }
        }

        div { class: "card",
            div { class: "card-header", div { class: "card-title", {crate::i18n::t("data_next_expiry")} } }
            div { class: "card-body space-y-1",
                if next.is_empty() {
                    p { class: "text-sm text-muted-foreground", "{empty_lbl}" }
                } else {
                    ul { class: "space-y-1",
                        for (id, cls, label) in next_views {
                            li { class: "flex items-center justify-between rounded border border-border p-2 text-sm",
                                span { class: "font-mono text-xs", "#{id}" }
                                span { class: "{cls} tabular", "expires {label}" }
                            }
                        }
                    }
                }
            }
        }

        div { class: "card",
            div { class: "card-header",
                div { class: "card-title", "{tombs_lbl}" }
                span { class: "text-sm text-muted-foreground", "{tomb_rows.len()}" }
            }
            div { class: "card-body",
                if tomb_rows.is_empty() {
                    p { class: "text-sm text-muted-foreground", "{empty_lbl}" }
                } else {
                    ul { class: "space-y-1",
                        for (id, reason) in tomb_rows {
                            li { class: "flex items-center justify-between rounded border border-border p-2 text-sm",
                                span { class: "font-mono text-xs", "#{id}" }
                                span { class: "text-muted-foreground text-xs", "{reason}" }
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
                None => rsx! { span { class: "text-muted-foreground", "{status_lbl}" } },
            }
        }
    }
}

/// v1.28.1 M4 (F-12) pure: parse the purge ids input exactly as `run_purge`
/// does (space/comma/newline separated; silently ignores unparseable tokens —
/// same contract the old raw-click button had).
pub fn parse_purge_ids(input: &str) -> Vec<i64> {
    input
        .split([' ', ',', '\n'])
        .filter(|s| !s.trim().is_empty())
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

/// v1.28.1 M4 (F-12) pure: is the rendered purge preview still current? The
/// preview card snapshots the ids/owner input at render time; an edit since
/// then leaves the snapshot stale and the destructive confirm must freeze
/// until a fresh preview is rendered.
pub fn purge_preview_fresh(
    input_ids: &str,
    input_owner: &str,
    snap_ids: &str,
    snap_owner: &str,
) -> bool {
    input_ids == snap_ids && input_owner == snap_owner
}

/// v1.20.22 M2.2: the "what expires next" pure core — sorts by expiry, caps at
/// 10, and skips already-expired rows (the server excludes them anyway; the
/// boundary lives here so a near-expiry row renders with its countdown label).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::DecayedRow;

    fn row(id: i64, effective: Option<i64>, expires: Option<i64>) -> DecayedRow {
        DecayedRow {
            id,
            content_hash: None,
            expires_at: expires,
            effective_expiry: effective,
            memory_kind: "fact".into(),
            reason: "kind_policy".into(),
        }
    }

    // ── v1.28.1 "Holdall" M4 (F-12): purge requires a confirmed preview ──

    #[test]
    fn purge_requires_confirmed_preview() {
        // The parse contract matches the purge path's own split.
        assert_eq!(parse_purge_ids("1, 2 3\n4"), vec![1, 2, 3, 4]);
        assert!(parse_purge_ids("abc, 12").is_empty() || parse_purge_ids("abc, 12") == vec![12]);

        // Fresh preview: snapshot equals the current input → confirm allowed.
        assert!(purge_preview_fresh("1,2", "o@c", "1,2", "o@c"));
        // ANY edit since the preview (ids OR owner) freezes the confirm.
        assert!(!purge_preview_fresh("1,2", "o@c", "1,2", "o@b"));
        assert!(!purge_preview_fresh("1,3", "o@c", "1,2", "o@c"));
        // No preview at all → frozen (the gate the button's blocked flag reads).
        assert!(!purge_preview_fresh("1,2", "o@c", "", ""));

        // The shared component gate: armed + fresh + enabled → fire; armed but
        // stale preview → confirm stays impossible until a fresh card renders.
        assert!(crate::confirm::confirm_allowed(true, false, false));
        assert!(!crate::confirm::confirm_allowed(true, true, false));
        assert!(!crate::confirm::confirm_allowed(false, false, false));
        // Unarmed + blocked → the button is inert (arm_allowed false).
        assert!(!crate::confirm::arm_allowed(false, true, false));
        assert!(crate::confirm::arm_allowed(false, false, false));
    }

    #[test]
    fn next_expiries_sorts_by_expiry_caps_at_ten_and_skips_expired() {
        let now = 1_000_000i64;
        let rows = vec![
            row(1, Some(now + 40), None),  // far
            row(2, Some(now + 30), None),  // mid
            row(3, Some(now + 10), None),  // soonest → first
            row(4, Some(now - 5), None),   // already expired → skipped
            row(5, Some(now - 100), None), // expired, no expiry → skipped
        ];
        let out = next_expiries(&rows, now);
        let ids: Vec<i64> = out.iter().map(|(i, _)| *i).collect();
        assert_eq!(
            ids,
            vec![3, 2, 1],
            "sorted ascending by expiry, expired skipped"
        );
        // Cap at 10.
        let many: Vec<DecayedRow> = (0..15)
            .map(|i| row(i as i64, Some(now + 1), None))
            .collect();
        assert_eq!(next_expiries(&many, now).len(), 10);
    }
}
