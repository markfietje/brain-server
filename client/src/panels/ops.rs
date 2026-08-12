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
use dioxus::prelude::*;

/// Mirrors the server's `BRAIN_PROPOSAL_TTL_SECS` default (7 days). The
/// server enforces expiry (`expire_if_stale`); this is the *display* deadline.
/// ponytail: if an operator overrides the server TTL this constant drifts —
/// a client can't read the server's env, so it is a documented mirror of the
/// shipped default. The server's 400 on a stale approve is the backstop.
pub const DEFAULT_PROPOSAL_TTL_SECS: u64 = 7 * 24 * 3600;

/// `Some(secs)` until the proposal expires, `None` once past its deadline.
/// The single source of truth for every countdown (M2).
pub fn clock_until(created_at: i64, ttl_secs: u64, now_unix: i64) -> Option<u64> {
    let deadline = created_at.saturating_add(ttl_secs as i64);
    if now_unix >= deadline {
        None
    } else {
        Some((deadline - now_unix) as u64)
    }
}

/// SLA tier for a remaining countdown (the 2026 SLA-per-state guidance mapped
/// onto the 7d TTL): `critical` < 5 min, `warn` < 1 hr, else `ok`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Critical,
    Warn,
    Ok,
}

pub fn sla_tier(remaining_secs: u64) -> Tier {
    if remaining_secs < 5 * 60 {
        Tier::Critical
    } else if remaining_secs < 3600 {
        Tier::Warn
    } else {
        Tier::Ok
    }
}

/// Compact "Xd Yh / Xh Ym / Xm Ys / Xs" countdown label (never bare "pending").
pub fn fmt_remaining(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

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
/// nearest-expiry, stable tie-break by id (no server ordering change).
pub fn queue_priority(rows: &mut [Proposal], ttl: u64, now_unix: i64) {
    rows.sort_by_key(|p| (clock_until(p.created_at, ttl, now_unix).unwrap_or(0), p.id));
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn panel() -> Element {
    use_document_title(|| "Operations — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<crate::UiState>();
    let writes = (ui.writes_enabled)(); // read once; re-renders when it changes
    let refresh = use_signal(|| 0u32); // bump to refetch after a mutation
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
    queue_priority(&mut ordered, DEFAULT_PROPOSAL_TTL_SECS, now);

    // M3 decide — approve/reject reuse the same server call + offline-enqueue
    // replay as review (v1.20.0 posture), then refetch.
    let decide = move |id: i64, reject: bool| {
        let api = api();
        let mut refresh = refresh;
        spawn(async move {
            let res: Result<(), crate::api::ApiError> = if reject {
                api.reject_proposal(id, None).await.map(|_| ())
            } else {
                api.approve_proposal(id, None).await.map(|_| ())
            };
            if let Err(e) = res {
                if crate::queue::is_offline(&e) {
                    crate::queue::enqueue(if reject {
                        crate::queue::QueuedAction::Reject { id, reason: None }
                    } else {
                        crate::queue::QueuedAction::Approve {
                            id,
                            supersedes: None,
                        }
                    });
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
        let expired = ordered
            .iter()
            .filter(|p| clock_until(p.created_at, DEFAULT_PROPOSAL_TTL_SECS, now).is_none())
            .count() as u64;
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
                                p { class: "text-sm text-foreground mt-1", {crate::strip_invisible(&p.content)} }
                                if let Some(sp) = &p.source_prompt {
                                    details { class: "mt-1 text-xs",
                                        summary { class: "cursor-pointer text-accent", {crate::i18n::t("ops_sourcing")} }
                                        p { class: "mt-1 text-ink-faint whitespace-pre-wrap border border-border rounded p-2", {sp.clone()} }
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

/// The per-row SLA countdown: a tier-colored badge, or an expired marker.
/// Inlined (not `#[component]`) — the `p` borrow is a `&Proposal` reference.
fn clock_badge(p: &Proposal, now: i64) -> Element {
    match clock_until(p.created_at, DEFAULT_PROPOSAL_TTL_SECS, now) {
        Some(remaining) => {
            let (cls, key) = match sla_tier(remaining) {
                Tier::Critical => ("badge-danger", "sla_critical"),
                Tier::Warn => ("badge-warn", "sla_warn"),
                Tier::Ok => ("badge-ok", "sla_remaining"),
            };
            let label = if key == "sla_remaining" {
                fmt_remaining(remaining)
            } else {
                crate::i18n::t(key)
            };
            rsx! {
                span { class: "badge {cls} tabular", title: "time until expiry", "{label}" }
            }
        }
        None => rsx! {
            span { class: "badge badge-danger tabular", title: "server auto-rejects expired proposals",
                {crate::i18n::t("ops_expired")} }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86400;
    const TTL: u64 = 7 * DAY;

    fn prop(id: i64, created_at: i64) -> Proposal {
        Proposal {
            id,
            kind: "memory".into(),
            content: "c".into(),
            source: None,
            source_prompt: None,
            screen_verdict: None,
            authority: None,
            novelty: 0.5,
            conflict_with: None,
            salience: 0.5,
            created_at,
            edited_at: None,
        }
    }

    #[test]
    fn clock_until_returns_remaining_and_none_when_expired() {
        assert_eq!(clock_until(1000, TTL, 1000), Some(TTL));
        assert_eq!(clock_until(1000, TTL, 1000 + 3600), Some(TTL - 3600));
        assert_eq!(clock_until(1000, TTL, 1000 + TTL as i64), None); // at the deadline
        assert_eq!(clock_until(1000, TTL, 1000 + TTL as i64 + 1), None); // past it
    }

    #[test]
    fn sla_tier_maps_budgets() {
        assert_eq!(sla_tier(0), Tier::Critical);
        assert_eq!(sla_tier(299), Tier::Critical);
        assert_eq!(sla_tier(300), Tier::Warn); // 5 min boundary → warn
        assert_eq!(sla_tier(3599), Tier::Warn);
        assert_eq!(sla_tier(3600), Tier::Ok); // 1 hr boundary → ok
        assert_eq!(sla_tier(7 * DAY), Tier::Ok);
    }

    #[test]
    fn fmt_remaining_labels() {
        assert_eq!(fmt_remaining(45), "45s");
        assert_eq!(fmt_remaining(125), "2m 5s");
        assert_eq!(fmt_remaining(3661), "1h 1m");
        assert_eq!(fmt_remaining(2 * DAY + 3600), "2d 1h");
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
        // remaining = created_at + TTL - now, so within the window an OLDER
        // row has an earlier deadline → nearest expiry → sorts first.
        let now = 5000;
        let mut rows = vec![
            prop(1, 0),                    // remaining TTL-5000 (nearest in-window) → first
            prop(2, 1000),                 // remaining TTL-4000 → next
            prop(3, 2000),                 // remaining TTL-3000 → last in-window
            prop(4, now - TTL as i64 - 1), // past deadline → expired → first overall
        ];
        queue_priority(&mut rows, TTL, now);
        let ids: Vec<i64> = rows.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![4, 1, 2, 3]);
    }

    #[test]
    fn queue_priority_stable_tie_break_by_id() {
        // Two rows with identical created_at → equal remaining; id ascending.
        let mut rows = vec![prop(9, 1000), prop(3, 1000), prop(7, 1000)];
        queue_priority(&mut rows, TTL, 5000);
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
}
