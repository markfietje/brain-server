//! Review panel — the approval queue (DESIGN §4.1). Context-rich cards showing
//! the *why* (novelty / conflict / salience), not binary buttons. The default
//! landing panel.
//!
//! v1.16.0 M3: honest batch partial-failure (per-row `RowOutcome` — a failed
//! call in the batch is surfaced, never silently dropped), keyboard-first
//! (`A`/`S`/`R`/`J`/`K` with a 2.1.4 shortcuts toggle), reject-with-reason, and
//! suggest-re-ingest. The connection mutation freeze (M1) disables the buttons
//! when `writes_enabled` is false.

use crate::api::{error_message, ApiClient, ApiError, Proposal};
use crate::panels::{use_document_title, PageTitle, RefreshButton};
use crate::{Route, UiState};
use dioxus::prelude::*;
use std::collections::{HashMap, HashSet};

/// M3.1: per-row outcome. A failed call in a batch is surfaced here, not
/// silently dropped. `AlreadyDone` treats a 404-no-pending as success (the
/// approve/reject contract is non-idempotent — DESIGN §4.1). `Queued` (v1.20.0
/// M3) is a write that hit the offline window and awaits replay.
#[derive(Clone, Debug, PartialEq)]
pub enum RowOutcome {
    Pending,
    Done(i64),
    AlreadyDone,
    Queued,
    Failed(String),
}

/// M3 pure: classify a single approve/reject result into a `RowOutcome`.
/// `404-no-pending` → `AlreadyDone` (the id was already decided → success);
/// any other error → `Failed(reason)`. Extracted so the batch loop is plumbing.
fn classify_outcome(result: Result<i64, ApiError>) -> RowOutcome {
    match result {
        Ok(chunk_id) => RowOutcome::Done(chunk_id),
        Err(ApiError::Status(404, _)) => RowOutcome::AlreadyDone,
        Err(e) => RowOutcome::Failed(e.to_string()),
    }
}

/// v1.20.0 M3: settle one approve/reject result. An offline call enqueues the
/// action for replay (idempotency by key) and surfaces `Queued` instead of a
/// false failure; everything else falls through to `classify_outcome`.
fn settle(result: Result<i64, ApiError>, action: crate::queue::QueuedAction) -> RowOutcome {
    match result {
        Err(e) if crate::queue::is_offline(&e) => {
            crate::queue::enqueue(action);
            RowOutcome::Queued
        }
        other => classify_outcome(other),
    }
}

/// v1.20.15 "Clock": the queue-is-a-clock sort — nearest `expires_at` first
/// (expired dead-first), stable id tie-break. Pure so the ordering is pinned
/// without a Dioxus runtime. The key is the server-authoritative `expires_at`
/// (created + TTL), so an operator's `BRAIN_PROPOSAL_TTL_SECS` override is
/// respected with no client TTL mirror.
fn expiry_order(rows: &mut [Proposal]) {
    rows.sort_by_key(|p| (p.expires_at, p.id));
}

/// M3 pure (DropGuard logic): clear `Pending` rows from the selection so a
/// cancelled batch doesn't strand a half-applied selection. Pending rows
/// become re-selectable; Done/Failed/AlreadyDone stay as-is (already settled).
fn clear_pending_selection(
    outcomes: &HashMap<i64, RowOutcome>,
    selected: &HashSet<i64>,
) -> HashSet<i64> {
    selected
        .iter()
        .copied()
        .filter(|id| !matches!(outcomes.get(id), Some(RowOutcome::Pending)))
        .collect()
}

/// M3 pure: summarize a batch's per-row outcomes (v1.16.2 M6 — the listed
/// `batch_outcome`). Drives the batch-finished UI: counts + whether any row
/// failed so the panel can surface partial failure honestly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BatchSummary {
    pub done: usize,
    pub already_done: usize,
    pub queued: usize,
    pub failed: usize,
    pub pending: usize,
}

pub fn batch_outcome(outcomes: &HashMap<i64, RowOutcome>) -> BatchSummary {
    let mut s = BatchSummary::default();
    for o in outcomes.values() {
        match o {
            RowOutcome::Done(_) => s.done += 1,
            RowOutcome::AlreadyDone => s.already_done += 1,
            RowOutcome::Queued => s.queued += 1,
            RowOutcome::Failed(_) => s.failed += 1,
            RowOutcome::Pending => s.pending += 1,
        }
    }
    s
}

/// v1.16.2 M6: render the batch summary line once a batch has run (rows
/// settled out of Pending). Surfaces partial failure honestly.
fn batch_summary(outcomes: &HashMap<i64, RowOutcome>) -> Element {
    let s = batch_outcome(outcomes);
    if (s.done + s.already_done + s.queued + s.failed) > 0 && s.pending == 0 {
        rsx! {
            p { class: "text-xs text-muted-foreground mb-1", role: "status", "aria-live": "polite",
                "batch: {s.done} approved · {s.already_done} already decided · {s.queued} queued (offline) · {s.failed} failed"
            }
        }
    } else {
        rsx! {}
    }
}

/// v1.20.23 "Calibrate": the reviewer's own decision numbers over a `since`
/// window — the anti-rubber-stamp feedback loop, pure arithmetic over the wire
/// rows (zero new server logic). All rates are `[0,1]`; a zero denominator is
/// `0.0` (never a NaN — the v1.9.0 suggest-metrics lesson).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Calibration {
    /// Decisions (approved + rejected) in the window.
    pub decisions: usize,
    /// approved / (approved + rejected).
    pub approve_rate: f64,
    /// median(`decided_at - created_at`) over every decision with both stamps.
    pub median_latency_secs: Option<i64>,
    /// approved-with-`edited_at` / approved.
    pub edit_rate: f64,
    /// approved-with-`quarantine` read-time verdict / approved.
    pub override_rate: f64,
    /// The fetch hit the `limit=200` cap, so the window is a sampled prefix
    /// (the honest "last 200" label).
    pub capped: bool,
}

/// v1.20.23 "Calibrate" M2: the pure calibration core. `approved`/`rejected`
/// are the windowed fetches; the four signals fall out. Extracted so the
/// arithmetic is pinned without a Dioxus runtime (the `expiry_order` idiom).
pub fn calibration_stats(approved: &[Proposal], rejected: &[Proposal]) -> Calibration {
    let decisions = approved.len() + rejected.len();
    let approve_rate = if decisions == 0 {
        0.0
    } else {
        approved.len() as f64 / decisions as f64
    };
    // Latency per decision (either outcome) that has both stamps.
    let mut latencies: Vec<i64> = approved
        .iter()
        .chain(rejected.iter())
        .filter_map(|p| match (p.created_at, p.decided_at) {
            (c, Some(d)) => Some(d - c),
            _ => None,
        })
        .collect();
    latencies.sort_unstable();
    let median_latency_secs = match latencies.len() {
        0 => None,
        1 => Some(latencies[0]),
        n if n % 2 == 1 => Some(latencies[n / 2]),
        n => Some((latencies[n / 2 - 1] + latencies[n / 2]) / 2),
    };
    let n = approved.len();
    let edited = approved.iter().filter(|p| p.edited_at.is_some()).count();
    let overridden = approved
        .iter()
        .filter(|p| p.screen_verdict.as_deref() == Some("quarantine"))
        .count();
    Calibration {
        decisions,
        approve_rate,
        median_latency_secs,
        edit_rate: if n == 0 {
            0.0
        } else {
            edited as f64 / n as f64
        },
        override_rate: if n == 0 {
            0.0
        } else {
            overridden as f64 / n as f64
        },
        capped: n == 200 || rejected.len() == 200,
    }
}

/// v1.20.23 M2.1: the rubber-stamp warning condition — a near-uniform approval
/// over a meaningful decision count. A pure heuristic (a reviewer baseline is
/// v2.x cohort tooling); the threshold is the plan's 0.9 / 20 constant.
fn rubber_stamp(c: &Calibration) -> bool {
    c.approve_rate > 0.9 && c.decisions >= 20
}

/// v1.20.23 M2.1: a human decision latency reading from secs (e.g. `12m 5s`).
/// Pure so the strip is deterministic.
fn format_latency(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// v1.20.23 M2.1: the reviewer calibration strip — four labelled figures +
/// the rubber-stamp reading, fetched once on mount/refresh. Renders nothing
/// when the fetch fails (the v1.20.0 offline posture — stats degrade, never an
/// error card). Dismissable per the plan. A plain fn (like `card`) — no
/// `#[component]`, so the closures capture signals without the macro's
/// Clone+PartialEq prop constraints.
fn calibration_strip(api: Signal<ApiClient>, refresh: Signal<u32>) -> Element {
    let mut dismissed = use_signal(|| false);
    let stats = use_resource(move || {
        let api = api();
        let _ = refresh(); // subscribe → rerun when the queue refreshes
        async move {
            let since = crate::time_budget::now_unix() - 7 * 24 * 3600;
            match (
                api.proposals_since("approved", since).await,
                api.proposals_since("rejected", since).await,
            ) {
                (Ok(a), Ok(r)) => Some(calibration_stats(&a, &r)),
                _ => None,
            }
        }
    });
    if dismissed() {
        return rsx! {};
    }
    let c = match stats.read().as_ref() {
        Some(Some(c)) => *c,
        _ => return rsx! {}, // loading or fetch failed → nothing
    };
    let tier_class = if rubber_stamp(&c) {
        "border-warn"
    } else {
        "border-border"
    };
    let stamp_hint = if rubber_stamp(&c) {
        Some(crate::i18n::t("cal_warn_high"))
    } else {
        None
    };
    let latency = c
        .median_latency_secs
        .map(format_latency)
        .unwrap_or_else(|| "—".to_string());
    let dec_count = c.decisions.to_string();
    let dec_label = crate::i18n::t("cal_decisions");
    let capped_note = if c.capped {
        format!(" ({})", crate::i18n::t("cal_last_200"))
    } else {
        String::new()
    };
    rsx! {
        div {
            class: "card mt-2 border-2 {tier_class}",
            role: "status", "aria-live": "polite",
            div { class: "card-body text-xs",
                div { class: "flex items-center justify-between",
                    h2 { class: "card-title text-sm", {crate::i18n::t("cal_title")} }
                    button {
                        class: "btn btn-ghost btn-sm",
                        "aria-label": "dismiss calibration",
                        onclick: move |_| dismissed.set(true),
                        "dismiss"
                    }
                }
                div { class: "grid grid-cols-2 sm:grid-cols-4 gap-2 mt-1",
                    {stat(
                        crate::i18n::t("cal_approve_rate"),
                        format!("{:.0}%", c.approve_rate * 100.0),
                    )}
                    {stat(crate::i18n::t("cal_latency"), latency)}
                    {stat(
                        crate::i18n::t("cal_edit_rate"),
                        format!("{:.0}%", c.edit_rate * 100.0),
                    )}
                    {stat(
                        crate::i18n::t("cal_override_rate"),
                        format!("{:.0}%", c.override_rate * 100.0),
                    )}
                }
                p { class: "text-muted-foreground mt-1 tabular",
                    "{dec_count} {dec_label}{capped_note}"
                }
                if let Some(hint) = stamp_hint {
                    p { class: "text-warn mt-1", "{hint}" }
                }
            }
        }
    }
}

/// v1.20.23 M2.1: one labelled figure in the calibration strip.
fn stat(label: String, value: String) -> Element {
    rsx! {
        div { class: "flex flex-col",
            span { class: "text-ink-faint", "{label}" }
            span { class: "text-foreground font-medium tabular", "{value}" }
        }
    }
}

/// M3 cancel-safety (DESIGN §6): clears `Pending` rows from the selection
/// when the batch future is dropped. A `DropGuard` for the `spawn` future so a
/// mid-flight cancel cannot strand a half-applied selection.
struct BatchGuard {
    selected: Signal<HashSet<i64>>,
    outcomes: Signal<HashMap<i64, RowOutcome>>,
}

impl Drop for BatchGuard {
    fn drop(&mut self) {
        let cleared = clear_pending_selection(&(self.outcomes)(), &(self.selected)());
        self.selected.set(cleared);
    }
}

/// M3.2: map a key to a Review action. `None` = unhandled / shortcuts off.
/// Pure so the keyboard handler is plumbing and the mapping is testable.
/// `keyboard_types::Key` represents chars as `Character(String)`.
fn key_action(key: &Key, has_conflict: bool) -> Option<ReviewKey> {
    let c = match key {
        Key::Character(c) => c.as_str(),
        _ => return None,
    };
    match c {
        "a" | "A" => Some(ReviewKey::Approve),
        "r" | "R" => Some(ReviewKey::Reject),
        "e" | "E" => Some(ReviewKey::Edit),
        "j" | "J" => Some(ReviewKey::Down),
        "k" | "K" => Some(ReviewKey::Up),
        "s" | "S" if has_conflict => Some(ReviewKey::ApproveSupersede),
        "?" => Some(ReviewKey::Help),
        _ => None,
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum ReviewKey {
    Approve,
    ApproveSupersede,
    Reject,
    /// v1.20.14 "Steer" M1: rewrite the focused proposal's content before
    /// deciding (a `reject`-without-reason alternative for a fixable draft).
    Edit,
    Up,
    Down,
    Help,
}

/// M1.4: the in-app help rows for the review shortcuts. Pure so the rendered
/// list and the `?` mapping share one source of truth (WCAG 3.2.6 help).
/// Keys resolve through i18n so the same table localizes.
fn keyboard_help() -> Vec<(&'static str, &'static str)> {
    vec![
        ("review_key_approve", "a"),
        ("review_key_supersede", "s"),
        ("review_key_reject", "r"),
        ("review_key_edit", "e"),
        ("review_key_next", "j"),
        ("review_key_prev", "k"),
    ]
}

pub fn panel() -> Element {
    use_document_title(|| "Review — brain".into());
    let api = use_context::<Signal<ApiClient>>();
    let mut ui = use_context::<UiState>();
    let writes = (ui.writes_enabled)(); // read once; re-renders when it changes
    let refresh = use_signal(|| 0u32); // bump to refetch after a mutation
    let mut selected = use_signal(HashSet::<i64>::new);
    let outcomes = use_signal(HashMap::<i64, RowOutcome>::new);
    let mut cursor = use_signal(|| None::<usize>); // M3.2: keyboard focus index
    let mut shortcuts = use_signal(|| true); // M3.2: 2.1.4 toggle, default on
    let mut show_help = use_signal(|| false); // M1.4: `?` toggles the help table
    let mut reject_for = use_signal(|| None::<i64>); // proposal id awaiting reason
    let reingest_for = use_signal(|| None::<(i64, String)>); // M3: (id, content) → editor
    let mut edit_for = use_signal(|| None::<(i64, String)>); // v1.20.14: (id, content) → editor
                                                             // v1.20.15 "Clock": the queue is a clock — nearest `expires_at` first
                                                             // (toggleable to creation order), and a ~30s tick re-renders the live
                                                             // deadline badges from a fresh `now_unix()` (the same honest approximation
                                                             // /ops uses; the server's 400 on a stale approve stays authoritative).
    let mut sort_expiry = use_signal(|| true);
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

    let proposals = use_resource(move || {
        let api = api();
        let _ = refresh(); // subscribe → rerun when refresh bumps
        async move { api.proposals("pending").await }
    });

    // The ordered id list drives both the cursor + the batch. With the expiry
    // sort on, `expires_at` ascending (id tie-break) — expired first, then the
    // most urgent deadline (the clock rule). Never touches the server data.
    let ordered: Vec<Proposal> = match &*proposals.read() {
        Some(Ok(list)) => {
            let mut v = list.clone();
            if sort_expiry() {
                expiry_order(&mut v);
            }
            v
        }
        _ => Vec::new(),
    };
    let all_ids: Vec<i64> = ordered.iter().map(|p| p.id).collect();
    let _ = tick(); // re-render the countdowns on each clock bump
    let now = crate::time_budget::now_unix();

    // v1.27.12 "ReviewArmour": snapshot the displayed-bytes fingerprint per id.
    // decide/batch read it and pass it back on approve so the server binds the
    // decision to exactly the bytes this panel rendered. A `use_signal` handle
    // (Copy) so both `move` closures capture it without a borrow conflict; set
    // only when the list actually changes to avoid a render loop.
    let mut digests = use_signal(HashMap::<i64, String>::new);
    {
        let m: HashMap<i64, String> = ordered
            .iter()
            .map(|p| (p.id, p.content_digest.clone()))
            .collect();
        if *digests.read() != m {
            digests.set(m);
        }
    }

    // M2.1: publish the pending count so the AppShell badge reflects reality.
    // `use_effect` runs after render; writes the signal the top bar reads.
    use_effect(move || {
        let n = match &*proposals.read() {
            Some(Ok(list)) => list.len() as u32,
            _ => 0,
        };
        ui.pending_count.set(n);
    });

    // M3.1: honest batch. Each id gets its own outcome; no blanket retry;
    // refresh happens only after every call resolves. A `BatchGuard` clears
    // `Pending` rows from the selection on drop (cancel-safety, DESIGN §6):
    // if the future is cancelled mid-run, half-applied selections don't strand.
    let run_batch = move |ids: Vec<i64>, reject: bool, reason: Option<String>| {
        let api = api();
        let mut outcomes = outcomes;
        let mut refresh = refresh;
        let guard_selected = selected;
        let guard_outcomes = outcomes;
        spawn(async move {
            let _guard = BatchGuard {
                selected: guard_selected,
                outcomes: guard_outcomes,
            };
            for id in &ids {
                outcomes.write().insert(*id, RowOutcome::Pending);
            }
            for id in ids {
                let res = if reject {
                    api.reject_proposal(id, reason.as_deref()).await.map(|_| 0)
                } else {
                    let d = digests().get(&id).cloned();
                    api.approve_proposal(id, None, d.as_deref())
                        .await
                        .map(|r| r.chunk_id)
                };
                let action = if reject {
                    crate::queue::QueuedAction::Reject {
                        id,
                        reason: reason.clone(),
                    }
                } else {
                    crate::queue::QueuedAction::Approve {
                        id,
                        supersedes: None,
                    }
                };
                outcomes.write().insert(id, settle(res, action));
            }
            refresh += 1;
        });
    };

    let decide = move |id: i64, supersedes: Option<i64>, reject: bool| {
        let api = api();
        let mut refresh = refresh;
        let mut selected = selected;
        let mut outcomes = outcomes;
        spawn(async move {
            outcomes.write().insert(id, RowOutcome::Pending);
            let res = if reject {
                api.reject_proposal(id, None).await.map(|_| 0)
            } else {
                let d = digests().get(&id).cloned();
                api.approve_proposal(id, supersedes, d.as_deref())
                    .await
                    .map(|r| r.chunk_id)
            };
            let action = if reject {
                crate::queue::QueuedAction::Reject { id, reason: None }
            } else {
                crate::queue::QueuedAction::Approve { id, supersedes }
            };
            outcomes.write().insert(id, settle(res, action));
            selected.write().remove(&id);
            refresh += 1;
        });
    };

    // Toggle one proposal's checkbox in the selection set (UI-only, reversible —
    // the one optimistic-UI carve-out DESIGN §1.7 permits).
    let toggle_sel = move |id: i64| {
        let mut s = selected();
        if !s.insert(id) {
            s.remove(&id);
        }
        selected.set(s);
    };

    // M3.2: keyboard handler. Acts on the cursor card. `shortcuts_enabled`
    // must be on (WCAG 2.1.4); Esc handled by the drawer when open.
    let key_ids = all_ids.clone();
    let ordered_keys = ordered.clone();
    let onkeydown = move |e: Event<KeyboardData>| {
        if !shortcuts() || key_ids.is_empty() {
            return;
        }
        let idx = cursor().unwrap_or(0);
        let id = key_ids.get(idx).copied();
        // The cursor maps into the *ordered* list (matches the rendered order).
        let focused = id.and_then(|id| ordered_keys.iter().find(|p| p.id == id).cloned());
        let has_conflict = focused.as_ref().and_then(|p| p.conflict_with).is_some();
        match key_action(&e.key(), has_conflict) {
            Some(ReviewKey::Down) => cursor.set(Some((idx + 1).min(key_ids.len() - 1))),
            Some(ReviewKey::Up) => cursor.set(Some(idx.saturating_sub(1))),
            Some(ReviewKey::Approve) => {
                if let Some(id) = id {
                    decide(id, None, false);
                }
            }
            Some(ReviewKey::ApproveSupersede) => {
                if let (Some(id), Some(p)) = (id, &focused) {
                    decide(id, p.conflict_with, false);
                }
            }
            Some(ReviewKey::Reject) => {
                if let Some(id) = id {
                    reject_for.set(Some(id));
                }
            }
            Some(ReviewKey::Edit) => {
                if let Some(p) = &focused {
                    edit_for.set(Some((p.id, p.content.clone())));
                }
            }
            Some(ReviewKey::Help) => show_help.set(!show_help()),
            None => {}
        }
    };

    rsx! {
        div { tabindex: "0", onkeydown,
            PageTitle { {crate::i18n::t("review_title")} }
            div { class: "flex gap-2 my-2 items-center flex-wrap",
                button {
                    class: "btn btn-outline btn-md",
                    disabled: !writes || all_ids.is_empty(),
                    onclick: move |_| { selected.set(all_ids.iter().copied().collect()); },
                    "Select visible ({all_ids.len()})"
                }
                button {
                    class: "btn btn-primary btn-md",
                    disabled: !writes || selected().is_empty(),
                    onclick: move |_| run_batch(selected().iter().copied().collect(), false, None),
                    {crate::i18n::t("approve")} " selected (" {selected().len().to_string()} ")"
                }
                button {
                    class: "btn btn-ghost btn-md",
                    onclick: move |_| selected.set(HashSet::new()),
                    "Clear"
                }
                // v1.20.15 "Clock": toggle the queue ordering — nearest expiry
                // first (the live clock rule) vs the server's creation order.
                button {
                    class: "btn btn-outline btn-md",
                    onclick: move |_| sort_expiry.set(!sort_expiry()),
                    if sort_expiry() { "expiry first" } else { "creation order" }
                }
                // M3.2: WCAG 2.1.4 — single-char shortcuts must be turn-offable.
                label { class: "flex items-center gap-1.5 text-xs text-muted-foreground ml-2",
                    input {
                        "type": "checkbox",
                        class: "accent-accent",
                        checked: shortcuts(),
                        onchange: move |e| shortcuts.set(e.value() == "true"),
                    }
                    "keys (A/S/R/J/K)"
                }
                // M1.4: discoverable shortcut help (WCAG 3.2.6); `?` toggles it too.
                button {
                    class: "btn btn-ghost btn-md",
                    "type": "button",
                    "aria-expanded": show_help(),
                    "aria-label": crate::i18n::t("review_help_toggle"),
                    onclick: move |_| show_help.set(!show_help()),
                    "?"
                }
                // v1.17.0 M2.4: portable refresh trigger (pull-to-refresh ceil).
                div { class: "ml-auto", RefreshButton { refresh } }
            }
            // M1.4: the help table — the `?`/button toggled in-app keyboard map.
            if show_help() {
                dl {
                    class: "text-xs text-muted-foreground my-2 border border-border rounded p-2",
                    role: "note",
                    dt { {crate::i18n::t("review_help")} }
                    {keyboard_help().iter().map(|(key, k)| rsx! {
                        div { class: "flex gap-2",
                            kbd { class: "font-mono border border-border rounded px-1", {*k} }
                            span { {crate::i18n::t(key)} }
                        }
                    })}
                }
            }
            // v1.16.2 M6: one-line batch summary — surfaces partial failure
            // honestly once a batch has run (rows settle out of Pending).
            { batch_summary(&outcomes()) }
            // v1.20.23 "Calibrate": the reviewer's own decision numbers above
            // the queue (the anti-rubber-stamp feedback loop). Fetch-failed →
            // renders nothing (offline degrade, never an error card).
            { calibration_strip(api, refresh) }
            { if ordered.is_empty() {
                match &*proposals.read() {
                    Some(Ok(_)) => rsx! {
                        div { class: "card mt-2",
                            div { class: "card-body text-center",
                                p { class: "text-muted-foreground", {crate::i18n::t("no_pending")} }
                                button {
                                    class: "btn btn-outline btn-md mt-3",
                                    onclick: move |_| async move {
                                        let mut refresh = refresh;
                                        let _ = api().propose("sample proposal — approve me to try the gate").await;
                                        refresh += 1;
                                    },
                                    "Ingest a sample proposal to try the gate"
                                }
                            }
                        }
                    },
                    Some(Err(e)) => rsx! { p { class: "text-danger mt-2", "queue failed: {error_message(&e)}" } },
                    None => rsx! { p { class: "text-muted-foreground mt-2", "…" } },
                }
            } else {
                rsx! {
                    ul { class: "divide-y divide-border",
                        for (i, p) in ordered.iter().enumerate() {
                            { card(
                                p.clone(),
                                selected(),
                                outcomes(),
                                i,
                                cursor(),
                                writes,
                                now,
                                decide,
                                toggle_sel,
                                reject_for,
                                reingest_for,
                                edit_for,
                            ) }
                        }
                    }
                }
            } }
            // M3: reject-with-reason + suggest-re-ingest. Modal-ish inline editors.
            if let Some(id) = reject_for() {
                RejectEditor { id, api, outcomes, refresh, reject_for }
            }
            if let Some((id, content)) = reingest_for() {
                ReingestEditor { id, initial: content, api, refresh, reingest_for }
            }
            // v1.20.14 "Steer" M1: rewrite a pending proposal before deciding.
            if let Some((id, content)) = edit_for() {
                EditEditor { id, initial: content, api, refresh, edit_for }
            }
        }
    }
}

/// M3.1: one context-rich approval card with inline per-row outcome + the
/// keyboard cursor ring. Inlined (not a `#[component]`) because the closures
/// (`decide`) capture signals and the `#[component]` macro requires Clone+
/// PartialEq props.
#[allow(clippy::too_many_arguments)]
fn card(
    proposal: Proposal,
    selected: HashSet<i64>,
    outcomes: HashMap<i64, RowOutcome>,
    index: usize,
    cursor: Option<usize>,
    writes: bool,
    now: i64,
    decide: impl Fn(i64, Option<i64>, bool) + Copy + 'static,
    mut toggle: impl FnMut(i64) + Copy + 'static,
    mut reject_for: Signal<Option<i64>>,
    mut reingest_for: Signal<Option<(i64, String)>>,
    mut edit_for: Signal<Option<(i64, String)>>,
) -> Element {
    let id = proposal.id;
    let checked = selected.contains(&id);
    let conflict = proposal.conflict_with;
    let outcome = outcomes.get(&id).cloned();
    let is_focused = cursor == Some(index);
    let ring = if is_focused {
        " ring-2 ring-accent"
    } else {
        ""
    };
    let content_for_reingest = proposal.content.clone();
    let content_for_edit = proposal.content.clone();
    // v1.20.15 "Clock": the live absolute-deadline badge. `expires_at` is
    // server-authoritative; `warn_secs`/`critical_secs` are the SLA bands.
    let remaining = crate::time_budget::remaining(proposal.expires_at, now);
    let tier = crate::time_budget::tier(remaining, proposal.warn_secs, proposal.critical_secs);
    let tier_class = match tier {
        crate::time_budget::Tier::Critical | crate::time_budget::Tier::Expired => {
            "badge badge-danger"
        }
        crate::time_budget::Tier::Warn => "badge badge-warn",
        crate::time_budget::Tier::Ok => "badge",
    };
    let expiry_label = crate::time_budget::format_remaining(remaining);
    rsx! {
        li { class: "py-2.5{ring}",
            label { class: "flex items-start gap-3",
                input {
                    class: "mt-1 accent-accent",
                    "type": "checkbox",
                    checked,
                    disabled: !writes,
                    onchange: move |_| toggle(id),
                    "aria-label": "select proposal {id}",
                }
                div { class: "flex-1",
                    div { class: "flex justify-between items-center gap-2",
                        Link {
                            class: "font-mono text-sm text-accent hover:underline text-left",
                            to: Route::ReviewDetail { proposal_id: id },
                            "proposal #{id} · {proposal.kind}"
                        }
                        span { class: "text-xs text-muted-foreground tabular",
                            "novelty {proposal.novelty:.2} · salience {proposal.salience:.2}" }
                        span { class: "{tier_class} tabular", title: "approve before the deadline",
                            "{expiry_label}" }
                        if let Some(lbl) = crate::panels::edited_label(proposal.edited_at) {
                            span { class: "badge badge-warn", "{lbl}" }
                        }
                    }
                    if let Some(c) = conflict {
                        p { class: "text-sm text-warn",
                            "conflicts with chunk #{c} — approve to supersede" }
                    }
                    // v1.20.24 "Sweep" (LITL fence): proposal content renders in
                    // a bounded scroll box (not a full-height <p>) so padded
                    // memory content cannot push the approve/reject buttons
                    // below the fold — scroll-evasion padding loses its
                    // target while the reviewer can still read everything in
                    // the box. ponytail: nothing is truncated or summarized —
                    // the human's full-view decision is preserved.
                    div { class: "mt-1 max-h-40 overflow-y-auto whitespace-pre-wrap rounded border border-border/50 p-2 text-sm text-foreground",
                        {crate::strip_invisible(&proposal.content)}
                    }
                    if let Some(sp) = &proposal.source_prompt {
                        details { class: "mt-1 text-xs",
                            summary { class: "cursor-pointer text-accent", "sourcing prompt" }
                            p { class: "mt-1 text-ink-faint whitespace-pre-wrap border border-border rounded p-2",
                                "{crate::strip_invisible(sp)}" }
                        }
                    }
                    div { class: "flex gap-2 mt-1.5 items-center flex-wrap",
                        button {
                            class: "btn btn-primary btn-sm",
                            disabled: !writes,
                            onclick: move |_| decide(id, None, false),
                            {crate::i18n::t("approve")}
                        }
                        if conflict.is_some() {
                            button {
                                class: "btn btn-outline btn-sm",
                                disabled: !writes,
                                onclick: move |_| decide(id, conflict, false),
                                {crate::i18n::t("approve")} " & supersede"
                            }
                        }
                        button {
                            class: "btn btn-outline btn-sm",
                            disabled: !writes,
                            onclick: move |_| reject_for.set(Some(id)),
                            {crate::i18n::t("reject")}
                        }
                        // M3: suggest re-ingest as a proposal with edits (no silent drop).
                        button {
                            class: "btn btn-ghost btn-sm",
                            onclick: move |_| reingest_for.set(Some((id, content_for_reingest.clone()))),
                            "suggest re-ingest"
                        }
                        // v1.20.14 "Steer" M1: rewrite the content in place before
                        // deciding (edit-then-approve), instead of reject + re-ingest.
                        button {
                            class: "btn btn-ghost btn-sm",
                            disabled: !writes,
                            onclick: move |_| edit_for.set(Some((id, content_for_edit.clone()))),
                            {crate::i18n::t("edit")}
                        }
                    }
                    // M3.1: inline per-row outcome. Honesty: a failed call shows
                    // its reason; AlreadyDone is success; Pending is in-flight.
                    if let Some(o) = outcome {
                        match o {
                            RowOutcome::Pending => rsx! {
                                span { class: "text-xs text-ink-faint", "…" }
                            },
                            RowOutcome::Done(cid) => rsx! {
                                span { class: "text-xs text-ok", "✓ approved → chunk #{cid}" }
                            },
                            RowOutcome::AlreadyDone => rsx! {
                                span { class: "text-xs text-muted-foreground", "already decided" }
                            },
                            RowOutcome::Queued => rsx! {
                                span { class: "text-xs text-warn", "queued (offline)" }
                            },
                            RowOutcome::Failed(e) => rsx! {
                                span { class: "text-xs text-danger", "failed: {e}" }
                            },
                        }
                    }
                }
            }
        }
    }
}

/// M3: reject-with-reason. The reason is recorded in the audit log so a
/// rejection isn't a silent drop (DESIGN §4.1). Esc cancels.
#[component]
fn RejectEditor(
    id: i64,
    api: Signal<ApiClient>,
    outcomes: Signal<HashMap<i64, RowOutcome>>,
    refresh: Signal<u32>,
    reject_for: Signal<Option<i64>>,
) -> Element {
    let mut reject_for = reject_for;
    let mut reason = use_signal(String::new);
    rsx! {
        div {
            class: "fixed inset-0 bg-surface-overlay/80 flex items-center justify-center p-4",
            role: "dialog", "aria-modal": "true", "aria-label": "reject with reason",
            onkeydown: move |e| if e.key() == Key::Escape { reject_for.set(None) },
            div { class: "card p-4 w-full max-w-md bg-popover",
                h2 { class: "card-title", "Reject proposal #{id}" }
                textarea {
                    class: "input w-full mt-3 text-sm min-h-20",
                    rows: "3",
                    placeholder: "reason (recorded in the audit log)…",
                    value: "{reason}",
                    oninput: move |e| reason.set(e.value()),
                }
                div { class: "flex gap-2 mt-3 justify-end",
                    button {
                        class: "btn btn-ghost btn-md",
                        onclick: move |_| reject_for.set(None),
                        "Cancel"
                    }
                    button {
                        class: "btn btn-destructive btn-md",
                        onclick: move |_| {
                            let api = api;
                            let mut outcomes = outcomes;
                            let mut refresh = refresh;
                            let r = reason().clone();
                            let mut reject_for = reject_for;
                            spawn(async move {
                                outcomes.write().insert(id, RowOutcome::Pending);
                                let reason = r.trim().to_string();
                                let reason_opt =
                                    if reason.is_empty() { None } else { Some(reason) };
                                let res = api().reject_proposal(id, reason_opt.as_deref()).await.map(|_| 0);
                                outcomes.write().insert(
                                    id,
                                    settle(
                                        res,
                                        crate::queue::QueuedAction::Reject {
                                            id,
                                            reason: reason_opt,
                                        },
                                    ),
                                );
                                refresh += 1;
                                reject_for.set(None);
                            });
                        },
                        {crate::i18n::t("reject")}
                    }
                }
            }
        }
    }
}

/// M3: suggest re-ingest as a proposal with edits. Pre-fills with the rejected
/// proposal's content; the operator edits + posts a NEW proposal. No silent drop.
#[component]
fn ReingestEditor(
    id: i64,
    initial: String,
    api: Signal<ApiClient>,
    refresh: Signal<u32>,
    reingest_for: Signal<Option<(i64, String)>>,
) -> Element {
    let mut reingest_for = reingest_for;
    let mut content = use_signal(|| initial);
    rsx! {
        div {
            class: "fixed inset-0 bg-surface-overlay/80 flex items-center justify-center p-4",
            role: "dialog", "aria-modal": "true", "aria-label": "suggest re-ingest",
            onkeydown: move |e| if e.key() == Key::Escape { reingest_for.set(None) },
            div { class: "card p-4 w-full max-w-md bg-popover",
                h2 { class: "card-title", "Re-ingest proposal #{id} as a new proposal" }
                textarea {
                    class: "input w-full mt-3 text-sm min-h-28",
                    rows: "5",
                    value: "{content}",
                    oninput: move |e| content.set(e.value()),
                }
                div { class: "flex gap-2 mt-3 justify-end",
                    button {
                        class: "btn btn-ghost btn-md",
                        onclick: move |_| reingest_for.set(None),
                        "Cancel"
                    }
                    button {
                        class: "btn btn-primary btn-md",
                        disabled: content().trim().is_empty(),
                        onclick: move |_| {
                            let api = api;
                            let mut refresh = refresh;
                            let c = content().clone();
                            let mut reingest_for = reingest_for;
                            spawn(async move {
                                let _ = api().propose(&c).await;
                                refresh += 1;
                                reingest_for.set(None);
                            });
                        },
                        "Post new proposal"
                    }
                }
            }
        }
    }
}

/// v1.20.14 "Steer" M1: rewrite a pending proposal's content in place (edit-
/// then-approve). The server re-scores deterministically and stamps `edited_at`
/// (the badge keys off it). A writer that both edits AND refetches keeps the
/// stale proposal only until the refresh re-lands; the audit records only
/// before/after hashes. Esc cancels. Mirrors the RejectEditor/ReingestEditor
/// modal idiom (no Radix DialogRoot in the client).
#[component]
fn EditEditor(
    id: i64,
    initial: String,
    api: Signal<ApiClient>,
    refresh: Signal<u32>,
    edit_for: Signal<Option<(i64, String)>>,
) -> Element {
    let mut edit_for = edit_for;
    let mut content = use_signal(|| initial);
    let feedback = use_signal(|| None::<String>);
    rsx! {
        div {
            class: "fixed inset-0 bg-surface-overlay/80 flex items-center justify-center p-4",
            role: "dialog", "aria-modal": "true", "aria-label": "edit",
            onkeydown: move |e| if e.key() == Key::Escape { edit_for.set(None) },
            div { class: "card p-4 w-full max-w-md bg-popover",
                h2 { class: "card-title", "Edit proposal #{id}" }
                textarea {
                    class: "input w-full mt-3 text-sm min-h-28",
                    rows: "5",
                    value: "{content}",
                    oninput: move |e| content.set(e.value()),
                }
                if let Some(fb) = feedback() {
                    p { class: "text-danger mt-2 text-sm", {fb} }
                }
                div { class: "flex gap-2 mt-3 justify-end",
                    button {
                        class: "btn btn-ghost btn-md",
                        onclick: move |_| edit_for.set(None),
                        {crate::i18n::t("cancel")}
                    }
                    button {
                        class: "btn btn-primary btn-md",
                        disabled: content().trim().is_empty(),
                        onclick: move |_| {
                            let api = api;
                            let mut refresh = refresh;
                            let c = content().clone();
                            let mut edit_for = edit_for;
                            let mut feedback = feedback;
                            spawn(async move {
                                match api().edit_proposal(id, &c).await {
                                    Ok(_) => {
                                        refresh += 1;
                                        edit_for.set(None);
                                    }
                                    Err(e) => feedback.set(Some(error_message(&e))),
                                }
                            });
                        },
                        {crate::i18n::t("save_edit")}
                    }
                }
            }
        }
    }
}

/// v1.16.7 M1: locate a proposal in a pending list by id. Pure so the
/// deep-link detail view can find its card without an extra server roundtrip
/// (there's no `GET /proposals/{id}`; the pending queue is bounded).
fn locate_proposal(list: &[Proposal], id: i64) -> Option<Proposal> {
    list.iter().find(|p| p.id == id).cloned()
}

/// v1.16.7 M1: the deep-linkable proposal detail (`/review/:proposal_id`).
/// Renders one card read-only + Approve/Reject, so a reviewer can share the
/// *specific* item. A proposal already decided by someone else is not in the
/// pending list → shown as "no longer pending", not an error.
pub fn detail(proposal_id: i64) -> Element {
    use_document_title(move || format!("Proposal #{proposal_id} — brain"));
    let api = use_context::<Signal<ApiClient>>();
    let proposals = use_resource(move || {
        let api = api();
        async move { api.proposals("pending").await }
    });
    // v1.20.15 "Clock": a ~30s tick keeps the deadline badge live on the detail.
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
    let found = match &*proposals.read() {
        Some(Ok(list)) => locate_proposal(list, proposal_id),
        _ => None,
    };
    let now = crate::time_budget::now_unix();
    let _ = tick();
    let deadline = found.as_ref().map(|p| {
        let remaining = crate::time_budget::remaining(p.expires_at, now);
        let tier = crate::time_budget::tier(remaining, p.warn_secs, p.critical_secs);
        let class = match tier {
            crate::time_budget::Tier::Critical | crate::time_budget::Tier::Expired => {
                "badge badge-danger"
            }
            crate::time_budget::Tier::Warn => "badge badge-warn",
            crate::time_budget::Tier::Ok => "badge",
        };
        (
            class.to_string(),
            crate::time_budget::format_remaining(remaining),
        )
    });
    rsx! {
        PageTitle { {format!("{} #{proposal_id}", crate::i18n::t("proposal"))} }
        p { class: "text-xs text-muted-foreground mb-3",
            Link { to: Route::Review {}, "← back to the review queue" } }
        match found {
            Some(p) => rsx! {
                div { class: "card",
                    div { class: "card-header",
                        h2 { class: "card-title", "Proposal #{proposal_id} · {p.kind}" }
                        if let Some(v) = p.screen_verdict.as_deref() {
                            span { class: "badge badge-{crate::panels::verdict_badge(v)}",
                                "screen: {crate::panels::verdict_label(v)}" }
                        }
                        if let Some(lbl) = crate::panels::edited_label(p.edited_at) {
                            span { class: "badge badge-warn", "{lbl}" }
                        }
                        if let Some((class, lbl)) = &deadline {
                            span { class: "{class} tabular", title: "approve before the deadline", "{lbl}" }
                        }
                    }
                    div { class: "card-body space-y-2",
                        p { class: "text-sm text-foreground", "{crate::strip_invisible(&p.content)}" }
                        p { class: "text-xs text-muted-foreground tabular",
                            "novelty {p.novelty:.2} · salience {p.salience:.2} · created {p.created_at}" }
                        if let Some(c) = p.conflict_with {
                            p { class: "text-sm text-warn", "conflicts with chunk #{c} — approve to supersede" }
                        }
                    }
                    div { class: "card-footer", DetailActions { api, proposal_id, digest: p.content_digest.clone() } }
                }
            },
            None => rsx! {
                div { class: "card",
                    div { class: "card-body text-muted-foreground",
                        p { "No pending proposal #{proposal_id} (already decided?)." } }
                }
            },
        }
    }
}

/// v1.16.7 M1: Approve/Reject for the deep-linked proposal. On success it
/// returns to the queue (the item is gone); on failure it shows the reason
/// inline rather than silently dropping.
#[component]
fn DetailActions(api: Signal<ApiClient>, proposal_id: i64, digest: String) -> Element {
    let writes = (use_context::<UiState>().writes_enabled)();
    let state = use_signal(String::new);
    let nav = navigator();
    let approve = move |_| {
        let mut state = state;
        let digest = digest.clone();
        spawn(async move {
            match api()
                .approve_proposal(proposal_id, None, Some(&digest))
                .await
            {
                Ok(_) => {
                    nav.replace(Route::Review {});
                }
                Err(e) if crate::queue::is_offline(&e) => {
                    crate::queue::enqueue(crate::queue::QueuedAction::Approve {
                        id: proposal_id,
                        supersedes: None,
                    });
                    state.set("queued — will replay when the connection returns".to_string());
                }
                Err(e) => state.set(error_message(&e)),
            }
        });
    };
    let reject = move |_| {
        let mut state = state;
        spawn(async move {
            match api().reject_proposal(proposal_id, None).await {
                Ok(_) => {
                    nav.replace(Route::Review {});
                }
                Err(e) if crate::queue::is_offline(&e) => {
                    crate::queue::enqueue(crate::queue::QueuedAction::Reject {
                        id: proposal_id,
                        reason: None,
                    });
                    state.set("queued — will replay when the connection returns".to_string());
                }
                Err(e) => state.set(error_message(&e)),
            }
        });
    };
    let gates = crate::role::role_allows(&api().roles(), "approve");
    let gatej = crate::role::role_allows(&api().roles(), "reject");
    rsx! {
        div { class: "flex gap-2 items-center flex-wrap",
            button { class: "btn btn-primary btn-md", disabled: !writes || !gates, onclick: approve, {crate::i18n::t("approve")} }
            button { class: "btn btn-destructive btn-md", disabled: !writes || !gatej, onclick: reject, {crate::i18n::t("reject")} }
            if !state().is_empty() { span { class: "text-danger text-sm", "{state}" } }
        }
    }
}

// ---------------------------------------------------------------------------
// M3 tests — the runnable checks for the honest-batch rules (DESIGN §4.1).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// A 404-no-pending is treated as AlreadyDone (success — the id was
    /// already decided). No retry is fired.
    #[test]
    fn batch_404_is_treated_as_already_done() {
        let outcome = classify_outcome(Err(ApiError::Status(404, "no pending".into())));
        assert_eq!(outcome, RowOutcome::AlreadyDone);
    }

    /// A success carries the chunk id; a 500 carries the reason. The batch
    /// surfaces partial failure honestly.
    #[test]
    fn batch_surfaces_partial_failure() {
        assert_eq!(classify_outcome(Ok(42)), RowOutcome::Done(42));
        let failed = classify_outcome(Err(ApiError::Status(500, "boom".into())));
        match failed {
            RowOutcome::Failed(msg) => assert!(msg.contains("500")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// The DropGuard logic: clearing Pending rows from the selection leaves
    /// settled rows untouched (Done/Failed/AlreadyDone stay selected).
    #[test]
    fn drop_guard_clears_pending_selection_on_cancel() {
        let mut outcomes = HashMap::new();
        outcomes.insert(1, RowOutcome::Pending);
        outcomes.insert(2, RowOutcome::Done(99));
        outcomes.insert(3, RowOutcome::Failed("x".into()));
        let mut selected = HashSet::new();
        selected.insert(1);
        selected.insert(2);
        selected.insert(3);
        let cleared = clear_pending_selection(&outcomes, &selected);
        assert!(!cleared.contains(&1), "Pending row must be cleared");
        assert!(cleared.contains(&2) && cleared.contains(&3));
    }

    /// Keyboard mapping: A/R/J/K always; S only when the focused card conflicts.
    #[test]
    fn keyboard_maps_asrjk_and_s_only_on_conflict() {
        assert_eq!(
            key_action(&Key::Character("a".into()), false),
            Some(ReviewKey::Approve)
        );
        assert_eq!(
            key_action(&Key::Character("R".into()), false),
            Some(ReviewKey::Reject)
        );
        assert_eq!(
            key_action(&Key::Character("j".into()), false),
            Some(ReviewKey::Down)
        );
        assert_eq!(
            key_action(&Key::Character("K".into()), false),
            Some(ReviewKey::Up)
        );
        // S only with a conflict; without one it's unhandled.
        assert_eq!(
            key_action(&Key::Character("s".into()), true),
            Some(ReviewKey::ApproveSupersede)
        );
        assert_eq!(key_action(&Key::Character("s".into()), false), None);
        // E opens the in-place content editor (v1.20.14 "Steer").
        assert_eq!(
            key_action(&Key::Character("e".into()), false),
            Some(ReviewKey::Edit)
        );
        // Unrelated keys are unhandled.
        assert_eq!(key_action(&Key::Character("z".into()), true), None);
        assert_eq!(key_action(&Key::Enter, true), None);
    }

    /// M1.4: `?` toggles the help table; the help table lists every mapped key
    /// (so a keyboard-only user can discover the shortcuts — WCAG 3.2.6).
    #[test]
    fn question_mark_opens_help_and_table_covers_all_keys() {
        assert_eq!(
            key_action(&Key::Character("?".into()), false),
            Some(ReviewKey::Help)
        );
        let mapped: Vec<&str> = ["a", "A", "r", "R", "j", "J", "k", "K"]
            .into_iter()
            .filter_map(|c| key_action(&Key::Character(c.into()), true))
            .map(|k| match k {
                ReviewKey::Help => "?",
                ReviewKey::Approve => "a",
                ReviewKey::ApproveSupersede => "s",
                ReviewKey::Reject => "r",
                ReviewKey::Up => "k",
                ReviewKey::Down => "j",
                ReviewKey::Edit => "e",
            })
            .collect();
        let shown: Vec<&str> = keyboard_help().iter().map(|(_, k)| *k).collect();
        for k in ["a", "s", "r", "j", "k", "e"] {
            assert!(shown.contains(&k), "help table missing '{k}'");
        }
        assert!(shown.contains(&"a"), "approve key documented");
        assert!(!mapped.is_empty());
    }

    /// v1.16.2 M6: `batch_outcome` summarizes the per-row outcomes so the
    /// batch UI can surface partial failure honestly.
    #[test]
    fn batch_outcome_counts_rows_and_flags_failure() {
        let mut outcomes = HashMap::new();
        outcomes.insert(1, RowOutcome::Done(10));
        outcomes.insert(2, RowOutcome::AlreadyDone);
        outcomes.insert(3, RowOutcome::Failed("boom".into()));
        outcomes.insert(4, RowOutcome::Pending);
        let s = batch_outcome(&outcomes);
        assert_eq!(
            s,
            BatchSummary {
                done: 1,
                already_done: 1,
                failed: 1,
                queued: 0,
                pending: 1,
            }
        );
        assert!(s.failed > 0, "a failed row must be surfaced");
        // An empty batch is all zeros.
        assert_eq!(batch_outcome(&HashMap::new()), BatchSummary::default());
    }

    /// v1.16.7 M1: the deep-link detail locates a proposal by id; an absent
    /// id is `None` (renders as "no longer pending"), not a panic.
    #[test]
    fn locate_proposal_finds_by_id_or_returns_none() {
        let list = vec![Proposal {
            id: 7,
            kind: "fact".into(),
            content: "x".into(),
            content_digest: String::new(),
            source: None,
            source_prompt: None,
            screen_verdict: None,
            authority: None,
            novelty: 0.1,
            conflict_with: None,
            salience: 0.2,
            created_at: 1,
            edited_at: None,
            expires_at: 1,
            warn_secs: 3600,
            critical_secs: 300,
            decided_at: None,
        }];
        assert_eq!(locate_proposal(&list, 7).map(|p| p.id), Some(7));
        assert_eq!(locate_proposal(&list, 99), None);
        assert_eq!(locate_proposal(&[], 7), None);
    }

    /// v1.20.15 "Clock": the queue-is-a-clock sort puts the most-urgent
    /// deadline (expired first) at the top, with a stable id tie-break.
    #[test]
    fn expiry_order_sorts_nearest_deadline_first() {
        fn prop(id: i64, created_at: i64) -> Proposal {
            Proposal {
                id,
                kind: "fact".into(),
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
                expires_at: created_at + 604800,
                warn_secs: 3600,
                critical_secs: 300,
                decided_at: None,
            }
        }
        let mut rows = vec![
            prop(3, 2000),              // expires 606800 → last
            prop(1, 0),                 // expires 604800 (nearest in-window) → first
            prop(2, 1000),              // expires 605800 → middle
            prop(4, 5000 - 604800 - 1), // expires 4999 → past deadline → first overall
        ];
        expiry_order(&mut rows);
        let ids: Vec<i64> = rows.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![4, 1, 2, 3]);

        // Identical deadlines → id ascending (stable tie-break).
        let mut ties = vec![prop(9, 1000), prop(3, 1000), prop(7, 1000)];
        expiry_order(&mut ties);
        assert_eq!(ties.iter().map(|p| p.id).collect::<Vec<_>>(), vec![3, 7, 9]);
    }

    /// v1.20.23 M2.2: a bounded fixture pins the exact rates + median — the
    /// arithmetic is pinned the same way `expiry_order` pins the sort.
    #[test]
    fn calibration_stats_rates_and_median() {
        fn prop(
            id: i64,
            created: i64,
            decided: Option<i64>,
            edited: Option<i64>,
            verdict: Option<&str>,
        ) -> Proposal {
            Proposal {
                id,
                kind: "fact".into(),
                content: "c".into(),
                content_digest: String::new(),
                source: None,
                source_prompt: None,
                screen_verdict: verdict.map(|s| s.to_string()),
                authority: None,
                novelty: 0.5,
                conflict_with: None,
                salience: 0.5,
                created_at: created,
                edited_at: edited,
                expires_at: created + 604800,
                warn_secs: 3600,
                critical_secs: 300,
                decided_at: decided,
            }
        }
        // 6 approved (latencies 10..60; #2 edited, #5 quarantined-verdict) +
        // 2 rejected (latencies 100, 200). Sorted latencies: 10,20,30,40,50,
        // 60,100,200 → median (4th+5th)/2 = (40+50)/2 = 45.
        let approved = vec![
            prop(1, 0, Some(10), None, None),
            prop(2, 0, Some(20), Some(5), None), // edited
            prop(3, 0, Some(30), None, None),
            prop(4, 0, Some(40), None, None),
            prop(5, 0, Some(50), None, Some("quarantine")), // screen override
            prop(6, 0, Some(60), None, None),
        ];
        let rejected = vec![
            prop(7, 0, Some(100), None, None),
            prop(8, 0, Some(200), None, None),
        ];
        let c = calibration_stats(&approved, &rejected);
        assert_eq!(c.decisions, 8);
        assert!(
            (c.approve_rate - 6.0 / 8.0).abs() < 1e-9,
            "approve_rate {:.3}",
            c.approve_rate
        );
        assert_eq!(c.median_latency_secs, Some(45));
        assert_eq!(c.edit_rate, 1.0 / 6.0);
        assert_eq!(c.override_rate, 1.0 / 6.0);
        assert!(!c.capped, "8 decisions under the 200 cap");
    }

    /// v1.20.23 M2.2: empty lists and zero denominators never NaN — rates `0.0`,
    /// latency `None`; decisions missing either stamp are skipped in the median.
    #[test]
    fn calibration_stats_handles_empty_and_zero_denominators() {
        fn prop(id: i64, created: i64, decided: Option<i64>) -> Proposal {
            Proposal {
                id,
                kind: "fact".into(),
                content: "c".into(),
                content_digest: String::new(),
                source: None,
                source_prompt: None,
                screen_verdict: None,
                authority: None,
                novelty: 0.5,
                conflict_with: None,
                salience: 0.5,
                created_at: created,
                edited_at: None,
                expires_at: created + 604800,
                warn_secs: 3600,
                critical_secs: 300,
                decided_at: decided,
            }
        }
        let empty = calibration_stats(&[], &[]);
        assert_eq!(empty.decisions, 0);
        assert_eq!(empty.approve_rate, 0.0);
        assert_eq!(empty.median_latency_secs, None);
        assert_eq!(empty.edit_rate, 0.0);
        assert_eq!(empty.override_rate, 0.0);

        // No approved → approve_rate 0, edit/override 0 (zero approved denom).
        let rej_only = calibration_stats(&[], &[prop(1, 0, Some(10))]);
        assert_eq!(rej_only.approve_rate, 0.0);
        assert_eq!(rej_only.edit_rate, 0.0);
        assert_eq!(
            rej_only.median_latency_secs,
            Some(10),
            "rejects still count a latency"
        );

        // A decision missing its `decided_at` (still pending, or no stamp) is
        // skipped in the median — never a bogus negative latency.
        let mixed = calibration_stats(
            &[prop(1, 0, Some(50)), prop(2, 0, None)],
            &[prop(3, 0, Some(70))],
        );
        assert_eq!(
            mixed.median_latency_secs,
            Some(60),
            "(50+70)/2, pending skipped"
        );
        assert_eq!(mixed.decisions, 3);
        assert_eq!(mixed.approve_rate, 2.0 / 3.0);
    }

    /// v1.20.23 M2.1: the rubber-stamp warn fires only over a real workload.
    #[test]
    fn rubber_stamp_warns_only_over_real_workload() {
        let warn = Calibration {
            decisions: 20,
            approve_rate: 0.93,
            ..Calibration::default()
        };
        assert!(rubber_stamp(&warn), ">0.9 over >=20 → warn");
        let few = Calibration {
            decisions: 10,
            approve_rate: 1.0,
            ..Calibration::default()
        };
        assert!(!rubber_stamp(&few), "too few decisions → no warn");
        let balanced = Calibration {
            decisions: 30,
            approve_rate: 0.8,
            ..Calibration::default()
        };
        assert!(!rubber_stamp(&balanced), "approve_rate <= 0.9 → no warn");
    }
}
