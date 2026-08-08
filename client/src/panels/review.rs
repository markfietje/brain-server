//! Review panel — the approval queue (DESIGN §4.1). Context-rich cards showing
//! the *why* (novelty / conflict / salience), not binary buttons. The default
//! landing panel.
//!
//! v1.16.0 M3: honest batch partial-failure (per-row `RowOutcome` — a failed
//! call in the batch is surfaced, never silently dropped), keyboard-first
//! (`A`/`S`/`R`/`J`/`K` with a 2.1.4 shortcuts toggle), reject-with-reason, and
//! suggest-re-ingest. The connection mutation freeze (M1) disables the buttons
//! when `writes_enabled` is false.

use crate::api::{ApiClient, ApiError, Proposal};
use crate::{DrawerContent, UiState};
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
            h1 { "Review queue" }
            div { class: "flex gap-2 my-2 items-center flex-wrap",
                button {
                    class: "border border-border-subtle surface-raised rounded px-2 py-1 text-sm disabled:opacity-50",
                    disabled: !writes || all_ids.is_empty(),
                    onclick: move |_| { selected.set(all_ids.iter().copied().collect()); },
                    "Select visible ({all_ids.len()})"
                }
                button {
                    class: "border border-border-subtle surface-raised rounded px-2 py-1 text-sm disabled:opacity-50",
                    disabled: !writes || selected().is_empty(),
                    onclick: move |_| run_batch(selected().iter().copied().collect(), false, None),
                    "Approve selected ({selected().len()})"
                }
                button {
                    class: "border border-border-subtle surface-raised rounded px-2 py-1 text-sm",
                    onclick: move |_| selected.set(HashSet::new()),
                    "Clear"
                }
                // M3.2: WCAG 2.1.4 — single-char shortcuts must be turn-offable.
                label { class: "flex items-center gap-1 text-xs text-ink-muted ml-2",
                    input {
                        "type": "checkbox",
                        checked: shortcuts(),
                        onchange: move |e| shortcuts.set(e.value() == "true"),
                    }
                    "keys (A/S/R/J/K)"
                }
            }
            match &*proposals.read() {
            Some(Ok(list)) if !list.is_empty() => rsx! {
                ul { class: "divide-y hairline",
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
                            ui,
                            reject_for,
                            reingest_for,
                        ) }
                    }
                }
            },
                Some(Ok(_)) => rsx! {
                    p { class: "text-ink-muted mt-2", "No pending proposals." }
                    button {
                        class: "border border-border-subtle surface-raised rounded px-2 py-1 text-sm mt-1",
                        onclick: move |_| async move {
                            let mut refresh = refresh;
                            let _ = api().propose("sample proposal — approve me to try the gate").await;
                            refresh += 1;
                        },
                        "Ingest a sample proposal to try the gate"
                    }
                },
                Some(Err(e)) => rsx! { p { class: "text-danger mt-2", "queue failed: {e}" } },
                None => rsx! { p { class: "text-ink-muted mt-2", "…" } },
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
    mut ui: UiState,
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
        li { class: "py-2{ring}",
            label { class: "flex items-start gap-2",
                input {
                    class: "mt-1",
                    "type": "checkbox",
                    checked,
                    disabled: !writes,
                    onchange: move |_| toggle(id),
                    "aria-label": "select proposal {id}",
                }
                div { class: "flex-1",
                    div { class: "flex justify-between",
                        button {
                            class: "font-mono text-sm text-accent hover:underline text-left",
                            onclick: move |_| ui.drawer.set(Some(DrawerContent::Proposal(proposal.clone()))),
                            "proposal #{id} · {proposal.kind}"
                        }
                        span { class: "text-xs text-ink-muted tabular",
                            "novelty {proposal.novelty:.2} · salience {proposal.salience:.2}" }
                    }
                    if let Some(c) = conflict {
                        p { class: "text-sm text-warn",
                            "conflicts with chunk #{c} — approve to supersede" }
                    }
                    p { class: "text-sm text-ink mt-1", "{proposal.content}" }
                    div { class: "flex gap-2 mt-1 items-center flex-wrap",
                        button {
                            class: "border border-border-subtle surface-raised rounded px-2 py-0.5 text-sm bg-accent text-white disabled:opacity-50",
                            disabled: !writes,
                            onclick: move |_| decide(id, None, false),
                            "Approve"
                        }
                        if conflict.is_some() {
                            button {
                                class: "border border-border-subtle surface-raised rounded px-2 py-0.5 text-sm disabled:opacity-50",
                                disabled: !writes,
                                onclick: move |_| decide(id, conflict, false),
                                "Approve & supersede"
                            }
                        }
                        button {
                            class: "border border-border-subtle surface-raised rounded px-2 py-0.5 text-sm disabled:opacity-50",
                            disabled: !writes,
                            onclick: move |_| reject_for.set(Some(id)),
                            "Reject"
                        }
                        // M3: suggest re-ingest as a proposal with edits (no silent drop).
                        button {
                            class: "text-xs text-ink-muted hover:text-accent",
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
                                span { class: "text-xs text-ink-muted", "already decided" }
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
            div { class: "surface-overlay border hairline rounded p-4 w-full max-w-md",
                h2 { class: "text-sm font-semibold", "Reject proposal #{id}" }
                textarea {
                    class: "border border-border-subtle surface-raised rounded px-2 py-1 w-full mt-2 text-sm",
                    rows: "3",
                    placeholder: "reason (recorded in the audit log)…",
                    value: "{reason}",
                    oninput: move |e| reason.set(e.value()),
                }
                div { class: "flex gap-2 mt-2 justify-end",
                    button {
                        class: "text-sm text-ink-muted px-2 py-1",
                        onclick: move |_| reject_for.set(None),
                        "Cancel"
                    }
                    button {
                        class: "border border-border-subtle rounded px-3 py-1 text-sm bg-danger text-white",
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
                        "Reject"
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
            div { class: "surface-overlay border hairline rounded p-4 w-full max-w-md",
                h2 { class: "text-sm font-semibold", "Re-ingest proposal #{id} as a new proposal" }
                textarea {
                    class: "border border-border-subtle surface-raised rounded px-2 py-1 w-full mt-2 text-sm",
                    rows: "5",
                    value: "{content}",
                    oninput: move |e| content.set(e.value()),
                }
                div { class: "flex gap-2 mt-2 justify-end",
                    button {
                        class: "text-sm text-ink-muted px-2 py-1",
                        onclick: move |_| reingest_for.set(None),
                        "Cancel"
                    }
                    button {
                        class: "border border-border-subtle rounded px-3 py-1 text-sm bg-accent text-white",
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
}
