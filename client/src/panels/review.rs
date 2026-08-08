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
/// approve/reject contract is non-idempotent — DESIGN §4.1).
#[derive(Clone, Debug, PartialEq)]
pub enum RowOutcome {
    Pending,
    Done(i64),
    AlreadyDone,
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
    pub failed: usize,
    pub pending: usize,
}

pub fn batch_outcome(outcomes: &HashMap<i64, RowOutcome>) -> BatchSummary {
    let mut s = BatchSummary::default();
    for o in outcomes.values() {
        match o {
            RowOutcome::Done(_) => s.done += 1,
            RowOutcome::AlreadyDone => s.already_done += 1,
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
    if (s.done + s.already_done + s.failed) > 0 && s.pending == 0 {
        rsx! {
            p { class: "text-xs text-muted-foreground mb-1", role: "status", "aria-live": "polite",
                "batch: {s.done} approved · {s.already_done} already decided · {s.failed} failed"
            }
        }
    } else {
        rsx! {}
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
        "j" | "J" => Some(ReviewKey::Down),
        "k" | "K" => Some(ReviewKey::Up),
        "s" | "S" if has_conflict => Some(ReviewKey::ApproveSupersede),
        _ => None,
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum ReviewKey {
    Approve,
    ApproveSupersede,
    Reject,
    Up,
    Down,
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
    let mut reject_for = use_signal(|| None::<i64>); // proposal id awaiting reason
    let reingest_for = use_signal(|| None::<(i64, String)>); // M3: (id, content) → editor

    let proposals = use_resource(move || {
        let api = api();
        let _ = refresh(); // subscribe → rerun when refresh bumps
        async move { api.proposals("pending").await }
    });

    // The ordered id list drives both the cursor + the batch.
    let all_ids: Vec<i64> = match &*proposals.read() {
        Some(Ok(list)) => list.iter().map(|p| p.id).collect(),
        _ => Vec::new(),
    };

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
                    api.approve_proposal(id, None).await.map(|r| r.chunk_id)
                };
                outcomes.write().insert(id, classify_outcome(res));
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
                api.approve_proposal(id, supersedes)
                    .await
                    .map(|r| r.chunk_id)
            };
            outcomes.write().insert(id, classify_outcome(res));
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
    let onkeydown = move |e: Event<KeyboardData>| {
        if !shortcuts() || key_ids.is_empty() {
            return;
        }
        let idx = cursor().unwrap_or(0);
        let id = key_ids.get(idx).copied();
        let focused = match &*proposals.read() {
            Some(Ok(list)) => list.get(idx).cloned(),
            _ => None,
        };
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
                // v1.17.0 M2.4: portable refresh trigger (pull-to-refresh ceil).
                div { class: "ml-auto", RefreshButton { refresh } }
            }
            // v1.16.2 M6: one-line batch summary — surfaces partial failure
            // honestly once a batch has run (rows settle out of Pending).
            { batch_summary(&outcomes()) }
            match &*proposals.read() {
            Some(Ok(list)) if !list.is_empty() => rsx! {
                ul { class: "divide-y divide-border",
                    for (i, p) in list.iter().enumerate() {
                        { card(
                            p.clone(),
                            selected(),
                            outcomes(),
                            i,
                            cursor(),
                            writes,
                            decide,
                            toggle_sel,
                            reject_for,
                            reingest_for,
                        ) }
                    }
                }
            },
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
            // M3: reject-with-reason + suggest-re-ingest. Modal-ish inline editors.
            if let Some(id) = reject_for() {
                RejectEditor { id, api, outcomes, refresh, reject_for }
            }
            if let Some((id, content)) = reingest_for() {
                ReingestEditor { id, initial: content, api, refresh, reingest_for }
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
    decide: impl Fn(i64, Option<i64>, bool) + Copy + 'static,
    mut toggle: impl FnMut(i64) + Copy + 'static,
    mut reject_for: Signal<Option<i64>>,
    mut reingest_for: Signal<Option<(i64, String)>>,
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
                    }
                    if let Some(c) = conflict {
                        p { class: "text-sm text-warn",
                            "conflicts with chunk #{c} — approve to supersede" }
                    }
                    p { class: "text-sm text-foreground mt-1", "{proposal.content}" }
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
                                let res = api().reject_proposal(id, if r.trim().is_empty() { None } else { Some(&r) }).await.map(|_| 0);
                                outcomes.write().insert(id, classify_outcome(res));
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
    let found = match &*proposals.read() {
        Some(Ok(list)) => locate_proposal(list, proposal_id),
        _ => None,
    };
    rsx! {
        PageTitle { {format!("{} #{proposal_id}", crate::i18n::t("proposal"))} }
        p { class: "text-xs text-muted-foreground mb-3",
            Link { to: Route::Review {}, "← back to the review queue" } }
        match found {
            Some(p) => rsx! {
                div { class: "card",
                    div { class: "card-header",
                        h2 { class: "card-title", "Proposal #{proposal_id} · {p.kind}" }
                    }
                    div { class: "card-body space-y-2",
                        p { class: "text-sm text-foreground", "{p.content}" }
                        p { class: "text-xs text-muted-foreground tabular",
                            "novelty {p.novelty:.2} · salience {p.salience:.2} · created {p.created_at}" }
                        if let Some(c) = p.conflict_with {
                            p { class: "text-sm text-warn", "conflicts with chunk #{c} — approve to supersede" }
                        }
                    }
                    div { class: "card-footer", DetailActions { api, proposal_id } }
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
fn DetailActions(api: Signal<ApiClient>, proposal_id: i64) -> Element {
    let writes = (use_context::<UiState>().writes_enabled)();
    let state = use_signal(String::new);
    let nav = navigator();
    let approve = move |_| {
        let mut state = state;
        spawn(async move {
            match api().approve_proposal(proposal_id, None).await {
                Ok(_) => {
                    nav.replace(Route::Review {});
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
                Err(e) => state.set(error_message(&e)),
            }
        });
    };
    rsx! {
        div { class: "flex gap-2 items-center flex-wrap",
            button { class: "btn btn-primary btn-md", disabled: !writes, onclick: approve, {crate::i18n::t("approve")} }
            button { class: "btn btn-destructive btn-md", disabled: !writes, onclick: reject, {crate::i18n::t("reject")} }
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
        // Unrelated keys are unhandled.
        assert_eq!(key_action(&Key::Character("z".into()), true), None);
        assert_eq!(key_action(&Key::Enter, true), None);
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
            source: None,
            authority: None,
            novelty: 0.1,
            conflict_with: None,
            salience: 0.2,
            created_at: 1,
        }];
        assert_eq!(locate_proposal(&list, 7).map(|p| p.id), Some(7));
        assert_eq!(locate_proposal(&list, 99), None);
        assert_eq!(locate_proposal(&[], 7), None);
    }
}
