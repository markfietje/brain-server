//! Memory Operations panel (v1.20.6 M1–M3). The HITL *work* surface — turns
//! the write gate + injection quarantine into something an operator runs day
//! to day. Three regions, one decision type each:
//!   1. Live pending queue (`GET /proposals?status=pending`) with a live SLA
//!      countdown clock per row (the "queue is a clock" rule). Expired /
//!      near-expiry rows surface first.
//!   2. Flagged & quarantined: recall with `include_flagged: true` (the
//!      injection screen's output) + `GET /decayed`, read-only, displayed via
//!      the shared invisible-char strip boundary (v1.20.3 G5).
//!   3. Gate health strip: approved / rejected / expired counts over a rolling
//!      window → a severity hint (the 2026 escalation-rate awareness).
//!
//! Pure client logic only — the panel is a composition of existing read
//! endpoints (`/proposals`, `/decayed`, `/recall?include_flagged`, `/health`);
//! no new wire types, no new server routes. The countdown is the one genuinely
//! new algorithm and lives in the Dioxus-free cores below.

use crate::api::{ApiClient, DecayedRow, Proposal};
use crate::panels::{use_document_title, PageTitle};
use crate::time_budget::{format_remaining, now_unix, remaining, tier, Tier};
use dioxus::prelude::*;

/// Gate-health severity + i18n label. Healthy when decisions are balanced;
/// `over-rejecting` when rejections exceed approvals; `under-reviewing` when
/// nothing has been decided but something is expiring on the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateHealth {
    pub severity: &'static str, // ok | warn | danger (semantic token)
    pub label: &'static str,    // i18n key
}

/// v1.20.8 M3: the /ops region an alert invalidates — `Pending` (the queue),
/// `Flagged` (the screen output), `Clock` (the SLA countdowns / expiry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Pending,
    Flagged,
    Clock,
}

/// v1.20.8 M3: map an alert kind to the region it invalidates. `pending` and
/// `chain` (audit-chain fail) refresh the queue; `screen` re-surfaces the
/// injection output; `expiry` restarts the countdown clock. Unknown kinds are
/// dropped (`None`) — the fixed, hand-curated server set is the contract.
pub fn region_for(kind: &str) -> Option<Region> {
    match kind {
        "pending" | "chain" => Some(Region::Pending),
        "screen" => Some(Region::Flagged),
        "expiry" => Some(Region::Clock),
        _ => None,
    }
}

/// v1.20.8 M3: the generation guard against a flood — apply an alert only when
/// its `seq` strictly advances past the last-seen one. A replay, dup, or an
/// out-of-order burst is dropped; the polling re-sync (and the every-mutation
/// refresh) cover any loss. `last` is updated in place.
pub fn should_apply(seq: u64, last: &mut u64) -> bool {
    if seq > *last {
        *last = seq;
        true
    } else {
        false
    }
}

/// v1.27.19 "Scrub" (D-7): the decide outcome → the gate-strip status line.
/// `None` (success) → nothing; offline network errors → the enqueue notice;
/// anything else → the raw server error, which the console now renders
/// instead of dropping. Pure so the surfacing rule is pinned by a test.
pub fn decide_status(res: &Result<(), crate::api::ApiError>) -> Option<Result<String, String>> {
    match res {
        Ok(()) => None,
        Err(e) if crate::queue::is_offline(e) => Some(Ok(crate::i18n::t("ops_queued_offline"))),
        Err(e) => Some(Err(crate::api::error_message(e))),
    }
}

pub fn gate_health(approved: u64, rejected: u64, expired: u64) -> GateHealth {
    if rejected > approved {
        GateHealth {
            severity: "danger",
            label: "gate_over_rejecting",
        }
    } else if approved == 0 && rejected == 0 && expired > 0 {
        GateHealth {
            severity: "warn",
            label: "gate_under_reviewing",
        }
    } else {
        GateHealth {
            severity: "ok",
            label: "gate_healthy",
        }
    }
}

/// Sort the pending queue in place for display: expired first, then
/// nearest-expiry, stable tie-break by id (no server ordering change). The
/// key is the server-authoritative `expires_at` (created + TTL) — no client
/// TTL mirror, so an operator's `BRAIN_PROPOSAL_TTL_SECS` override is respected.
pub fn queue_priority(rows: &mut [Proposal]) {
    rows.sort_by_key(|p| (p.expires_at, p.id));
}

pub fn panel() -> Element {
    use_document_title(|| "Operations — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<crate::UiState>();
    let writes = (ui.writes_enabled)(); // read once; re-renders when it changes
    let refresh = use_signal(|| 0u32); // bump to refetch after a mutation
                                       // v1.27.19 "Scrub" (D-7): last action/load outcome, rendered in the gate
                                       // strip — a failed decide/reject must be visible, not silently dropped.
    let status = use_signal(|| None::<Result<String, String>>);
    // M2: the live clock. A once-on-mount loop bumps `tick` every ~30s so every
    // countdown re-renders from a fresh `now_unix()` — the honest near-real-
    // time approximation (instant push is v1.20.8 "Signal").
    let tick = use_signal(|| 0u64);
    use_future(move || {
        let mut tick = tick;
        async move {
            loop {
                crate::probe_sleep(30).await;
                tick += 1;
            }
        }
    });

    // v1.20.8 M3: subscribe to the alert feed. A bounded `/events` read on a
    // ~10s interval (a real streaming EventSource needs a JS→Rust callback
    // bridge, which the eval-only web target doesn't have — this poll drains
    // the handshake + buffered alerts each connect and is the honest, testable
    // equivalent). On each applied alert, bump the matching region's refresh
    // signal once (the `should_apply` monotonic seq guard dedups a flood) and
    // set the aria-live announcement. If the feed is unreachable the 30s tick
    // poll above is the honest degrade — the console never goes silently stale.
    let alert_line = use_signal(|| "".to_string());
    let last_seq = use_signal(|| 0u64);
    use_future(move || {
        let api = api();
        let mut alert_line = alert_line;
        let mut last_seq = last_seq;
        let mut refresh = refresh;
        let mut tick = tick;
        async move {
            loop {
                crate::probe_sleep(10).await;
                let Ok(events) = api.alert_events().await else {
                    continue; // feed unreachable → the tick poll stands in
                };
                for e in events {
                    let mut last = last_seq();
                    if !should_apply(e.seq, &mut last) {
                        continue;
                    }
                    last_seq.set(last);
                    let region = region_for(&e.kind);
                    match region {
                        Some(Region::Pending) | Some(Region::Flagged) => refresh += 1,
                        Some(Region::Clock) => tick += 1,
                        None => {}
                    }
                    let msg = match region {
                        Some(Region::Pending) => crate::i18n::t("alert_queued"),
                        Some(Region::Flagged) => crate::i18n::t("alert_screen"),
                        Some(Region::Clock) => crate::i18n::t("alert_expiring"),
                        None => continue,
                    };
                    alert_line.set(msg);
                }
            }
        }
    });

    let proposals = use_resource(move || {
        let api = api();
        let _ = refresh(); // subscribe → rerun when refresh bumps
        let _ = tick();
        async move { api.proposals("pending").await }
    });
    let approved = use_resource(move || {
        let api = api();
        let _ = refresh();
        async move {
            api.proposals("approved")
                .await
                .map(|v| v.len() as u64)
                .unwrap_or(0)
        }
    });
    let rejected = use_resource(move || {
        let api = api();
        let _ = refresh();
        async move {
            api.proposals("rejected")
                .await
                .map(|v| v.len() as u64)
                .unwrap_or(0)
        }
    });

    // Region 2: the flagged surface — a probe recall (include_flagged) + the
    // decayed registry. `run` gates when it re-queries (explicit "Scan", not
    // per-keystroke).
    let mut probe = use_signal(String::new);
    let mut run = use_signal(|| 0u32);
    let flagged = use_resource(move || {
        let api = api();
        let _ = run();
        let _ = refresh();
        async move {
            let q = probe().trim().to_string();
            let rec = if q.len() >= crate::api::MIN_QUERY {
                api.recall(&q, false, None, true).await.ok()
            } else {
                None
            };
            let dec = api.decayed().await.unwrap_or_default();
            (rec, dec)
        }
    });

    let now = now_unix();
    let mut ordered: Vec<Proposal> = match &*proposals.read() {
        Some(Ok(list)) => list.clone(),
        _ => Vec::new(),
    };
    queue_priority(&mut ordered);

    // M3 decide — approve/reject reuse the same server call + offline-enqueue
    // replay as review (v1.20.0 posture), then refetch.
    let decide = move |id: i64, reject: bool| {
        let api = api();
        let mut refresh = refresh;
        let mut status = status;
        spawn(async move {
            let res: Result<(), crate::api::ApiError> = if reject {
                api.reject_proposal(id, None).await.map(|_| ())
            } else {
                api.approve_proposal(id, None, None).await.map(|_| ())
            };
            if let Err(ref e) = res {
                if crate::queue::is_offline(e) {
                    crate::queue::enqueue(if reject {
                        crate::queue::QueuedAction::Reject {
                            id,
                            queued_at: crate::queue::now_ts(),
                        }
                    } else {
                        crate::queue::QueuedAction::Approve {
                            id,
                            supersedes: None,
                            queued_at: crate::queue::now_ts(),
                        }
                    });
                }
                // v1.27.19 "Scrub" (D-7): was dropped silently (only the
                // offline branch surfaced anything). The non-offline failure
                // now renders in the gate strip.
                if let Some(s) = decide_status(&res) {
                    status.set(Some(s));
                }
            }
            refresh += 1;
        });
    };

    let (gh, a, r) = {
        let (a, r) = match (&*approved.read(), &*rejected.read()) {
            (Some(a), Some(r)) => (*a, *r),
            _ => (0, 0),
        };
        let expired = ordered.iter().filter(|p| p.expires_at <= now).count() as u64;
        (gate_health(a, r, expired), a, r)
    };

    let summary = format!("{} {}", ordered.len(), crate::i18n::t("ops_queue_summary"));
    let dec: Vec<DecayedRow> = match &*flagged.read() {
        Some((_, d)) => d.clone(),
        _ => Vec::new(),
    };

    rsx! {
        div {
            PageTitle { {crate::i18n::t("ops_title")} }
            p { class: "text-sm text-muted-foreground", {crate::i18n::t("ops_sub")} }

            // Region 3 (strip first — the top summary line).
            div { class: "card p-3 mt-3 flex items-center gap-3",
                span { class: "text-sm font-medium", {crate::i18n::t("ops_gate")} }
                span { class: "badge badge-{gh.severity}", {crate::i18n::t(gh.label)} }
                span { class: "text-xs text-muted-foreground tabular",
                    "A {a} · R {r}"
                }
                // v1.27.19 "Scrub" (D-7): the last decide/load outcome — the
                // console must never show a success state after a failed write.
                div { "role": "status", "aria-live": "polite", class: "text-sm",
                    match status() {
                        Some(Ok(m)) => rsx! { span { class: "text-ok", "{m}" } },
                        Some(Err(m)) => rsx! { span { class: "text-danger", "{m}" } },
                        None => rsx! {},
                    }
                }
            }

            // Region 1 — the live pending queue (top-left, the primary region).
            div { class: "card p-4 mt-3", "role": "region", "aria-label": crate::i18n::t("ops_queue"),
                div { class: "flex items-center justify-between",
                    h2 { class: "card-title", {crate::i18n::t("ops_queue")} }
                    span { class: "text-xs text-muted-foreground tabular", role: "status", "aria-live": "polite",
                        if !alert_line().is_empty() {
                            "{alert_line} · "
                        }
                        "{summary}"
                    }
                }
                match &*proposals.read() {
                    Some(Ok(_)) => rsx! { ul { class: "mt-2 divide-y divide-border",
                        for (pid, p) in ordered.iter().map(|p| (p.id, p)) {
                            li { class: "py-2.5",
                                div { class: "flex justify-between items-center gap-2",
                                    span { class: "font-mono text-sm text-accent", "proposal #{p.id} · {p.kind}" }
                                    span { class: "flex items-center gap-2",
                                        if let Some(v) = p.screen_verdict.as_deref() {
                                            span { class: "badge badge-{crate::panels::verdict_badge(v)}",
                                                "screen: {crate::panels::verdict_label(v)}" }
                                        }
                                        {clock_badge(p, now)}
                                    }
                                }
                                // v1.20.24 "Sweep" (LITL fence): bounded scroll
                                // box — same rationale as the review queue.
                                div { class: "mt-1 max-h-40 overflow-y-auto whitespace-pre-wrap rounded border border-border/50 p-2 text-sm text-foreground",
                                    {crate::strip_invisible(&p.content)}
                                }
                                if let Some(sp) = &p.source_prompt {
                                    details { class: "mt-1 text-xs",
                                        summary { class: "cursor-pointer text-accent", {crate::i18n::t("ops_sourcing")} }
                                        p { class: "mt-1 text-ink-faint whitespace-pre-wrap border border-border rounded p-2", {crate::strip_invisible(sp)} }
                                    }
                                }
                                div { class: "flex gap-2 mt-1.5 items-center",
                                    button {
                                        class: "btn btn-primary btn-sm",
                                        disabled: !writes,
                                        title: "approve (a)",
                                        onclick: move |_| decide(pid, false),
                                        {crate::i18n::t("approve")}
                                    }
                                    button {
                                        class: "btn btn-outline btn-sm",
                                        disabled: !writes,
                                        title: "reject (r)",
                                        onclick: move |_| decide(pid, true),
                                        {crate::i18n::t("reject")}
                                    }
                                }
                            }
                        }
                        }
                    },
                    Some(Err(e)) => rsx! { p { class: "text-danger mt-2", "queue failed: {e}" } },
                    None => rsx! { p { class: "text-muted-foreground mt-2", "…" } },
                }
            }

            // Region 2 — flagged & quarantined (the injection screen's output).
            div { class: "card p-4 mt-3", role: "region", "aria-label": crate::i18n::t("ops_flagged"),
                div { class: "flex items-center justify-between",
                    h2 { class: "card-title", {crate::i18n::t("ops_flagged")} }
                    div { class: "flex gap-2",
                        input {
                            class: "input input-sm",
                            placeholder: "probe query…",
                            value: "{probe}",
                            oninput: move |e| probe.set(e.value()),
                            "aria-label": "flagged probe query",
                        }
                        button { class: "btn btn-outline btn-sm", onclick: move |_| run += 1, {crate::i18n::t("ops_scan")} }
                    }
                }
                match &*flagged.read() {
                    Some((Some(rec), _)) => {
                        let hits: Vec<_> = rec.hits.iter().filter(|h| h.flagged == Some(true)).collect();
                        if hits.is_empty() {
                            rsx! { p { class: "text-muted-foreground mt-2", {crate::i18n::t("ops_flagged_empty")} } }
                        } else {
                            rsx! {
                                ul { class: "mt-2 divide-y divide-border",
                                    for h in &hits {
                                        li { class: "py-2",
                                            span { class: "badge badge-warn", "flagged" }
                                            p { class: "text-sm text-foreground mt-1", {crate::strip_invisible(&h.content)} }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    Some((None, _)) => rsx! { p { class: "text-muted-foreground mt-2", {crate::i18n::t("ops_flagged_hint")} } },
                    None => rsx! { p { class: "text-muted-foreground mt-2", "…" } },
                }
                if !dec.is_empty() {
                    div { class: "mt-3",
                        p { class: "text-xs font-medium text-muted-foreground", {crate::i18n::t("ops_decayed")} }
                        ul { class: "mt-1 divide-y divide-border",
                            for row in &dec {
                                li { class: "py-1.5 text-sm", "#{row.id} · {row.memory_kind}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The per-row SLA countdown: a tier-colored deadline label from the shared
/// clock core (`expires_at`-derived), or an expired marker.
/// Inlined (not `#[component]`) — the `p` borrow is a `&Proposal` reference.
fn clock_badge(p: &Proposal, now: i64) -> Element {
    let t = tier(remaining(p.expires_at, now), p.warn_secs, p.critical_secs);
    if t == Tier::Expired {
        return rsx! {
            span { class: "badge badge-danger tabular", title: "server auto-rejects expired proposals",
                {crate::i18n::t("ops_expired")} }
        };
    }
    let cls = match t {
        Tier::Critical => "badge-danger",
        Tier::Warn => "badge-warn",
        _ => "badge-ok",
    };
    rsx! {
        span { class: "badge {cls} tabular", title: "time until expiry",
            "expires in {format_remaining(remaining(p.expires_at, now))}" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(id: i64, created_at: i64) -> Proposal {
        Proposal {
            id,
            kind: "memory".into(),
            content: "c".into(),
            content_digest: String::new(),
            source: None,
            source_prompt: None,
            screen_verdict: None,
            authority: None,
            novelty: 0.5,
            conflict_with: None,
            salience: 0.5,
            created_at,
            edited_at: None,
            expires_at: created_at + 7 * 86400,
            warn_secs: 3600,
            critical_secs: 300,
            decided_at: None,
        }
    }

    #[test]
    fn gate_health_severity() {
        assert_eq!(gate_health(5, 2, 0).label, "gate_healthy");
        assert_eq!(gate_health(2, 5, 0).severity, "danger");
        assert_eq!(gate_health(2, 5, 0).label, "gate_over_rejecting");
        assert_eq!(gate_health(0, 0, 1).severity, "warn");
        assert_eq!(gate_health(0, 0, 1).label, "gate_under_reviewing");
    }

    #[test]
    fn queue_priority_expired_first_then_nearest_expiry() {
        // expires_at = created + TTL; an OLDER row has an earlier deadline →
        // nearest expiry → sorts first. A row past its deadline (expires_at <
        // now) leads.
        let now = 5000;
        let mut rows = vec![
            prop(1, 0),                   // expires 604800 (nearest in-window) → first
            prop(2, 1000),                // expires 605800 → next
            prop(3, 2000),                // expires 606800 → last in-window
            prop(4, now - 7 * 86400 - 1), // expires 4999 → past deadline → first overall
        ];
        queue_priority(&mut rows);
        let ids: Vec<i64> = rows.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![4, 1, 2, 3]);
    }

    #[test]
    fn queue_priority_stable_tie_break_by_id() {
        // Two rows with identical created_at → equal expires_at; id ascending.
        let mut rows = vec![prop(9, 1000), prop(3, 1000), prop(7, 1000)];
        queue_priority(&mut rows);
        let ids: Vec<i64> = rows.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![3, 7, 9]);
    }

    #[test]
    fn region_for_maps_alert_kinds_to_regions() {
        assert_eq!(region_for("pending"), Some(Region::Pending));
        assert_eq!(region_for("chain"), Some(Region::Pending));
        assert_eq!(region_for("screen"), Some(Region::Flagged));
        assert_eq!(region_for("expiry"), Some(Region::Clock));
        // Unknown kinds are dropped (the fixed server set is the contract).
        assert_eq!(region_for("nonsense"), None);
        assert_eq!(region_for(""), None);
    }

    #[test]
    fn should_apply_dedups_a_flood_and_accepts_advances() {
        let mut last = 0u64;
        assert!(should_apply(1, &mut last)); // first signal applies
        assert!(!should_apply(1, &mut last)); // replay dropped
        assert!(!should_apply(0, &mut last)); // out-of-order dropped
        assert!(should_apply(5, &mut last)); // next generation applies
        assert!(!should_apply(4, &mut last)); // older still dropped
        assert_eq!(last, 5);
    }

    /// v1.27.19 "Scrub" (D-7): a failed decide must surface an error status —
    /// the panel never renders success after a failed write.
    #[test]
    fn panel_write_errors_are_visible() {
        // Success → nothing to render.
        assert_eq!(decide_status(&Ok(())), None);
        // Any non-offline server error → the raw message, rendered as danger.
        let err = crate::api::ApiError::Status(500, "denylist write failed".into());
        let s = decide_status(&Err(err)).expect("server error must surface");
        assert!(s.is_err(), "a server failure is an Err status line");
        assert!(s.unwrap_err().contains("denylist write failed"));
        // 401 maps to the human session message, still an Err status line.
        let s = decide_status(&Err(crate::api::ApiError::Status(
            401,
            "unauthorized".into(),
        )))
        .expect("401 must surface");
        assert!(s.is_err());
        // Success after a queued write never renders danger.
        assert_eq!(decide_status(&Ok(())), None);
    }
}
