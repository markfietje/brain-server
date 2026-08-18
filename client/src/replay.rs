//! v1.28.1 "Holdall" M4 (F-34) — the destructive replay review. On reconnect
//! the queue auto-replays approve/reject/edit ONLY; `Purge`/`Dsar` actions
//! park here, behind an explicit per-item decision (Replay / Skip / Dismiss
//! all). No irreversible op ever auto-fires. Since v1.27.21 (N5) a third class
//! parks here too: an auto-replay that failed `MAX_REPLAY_RETRIES` times —
//! the banner is the dismissal surface, there is no further auto-retry.
//!
//! A parked `Dsar` persists only its subject's salted SHA-256 hash — replaying
//! RE-PROMPTS the subject and (v1.27.21 N2) verifies the retyped text against
//! the stored hash before the send button arms: a mistyped subject can never
//! erase the wrong person. A pre-salt legacy row (N7) cannot be verified, so
//! it demands only a non-empty retype. A parked `Purge` replays with the ids
//! + owner it carried (N8 — the owner is the erasure's scope).

use crate::api::ApiClient;
use crate::queue::QueuedAction;
use dioxus::prelude::*;

/// Pure: minutes since `queued_at` (ceil, floor at 0 → "just now").
pub fn queued_ago_minutes(queued_at: i64, now: i64) -> i64 {
    ((now - queued_at).max(0) + 59) / 60
}

/// v1.27.21 (N9): char-boundary-safe hash prefix — a corrupt stored hash can
/// hold multi-byte chars, and a byte-slice cut must never panic render.
fn hash_prefix(hash: &str) -> String {
    hash.chars().take(12).collect()
}

/// Pure: the one-line row summary — the scope the reviewer is deciding on.
/// Never includes the raw subject (only its hash prefix).
pub fn summary(action: &QueuedAction) -> String {
    match action {
        QueuedAction::Purge { chunk_ids, .. } => {
            let shown: Vec<String> = chunk_ids.iter().take(4).map(|i| format!("#{i}")).collect();
            let tail = if chunk_ids.len() > 4 {
                format!(" … +{}", chunk_ids.len() - 4)
            } else {
                String::new()
            };
            format!("{} ids {}{}", chunk_ids.len(), shown.join(", "), tail)
        }
        QueuedAction::Dsar {
            subject_hash,
            action: a,
            ..
        } => format!("{a} subject {}…", hash_prefix(subject_hash)),
        other => other.kind().to_string(),
    }
}

/// v1.27.21 (N2) pure: the DSAR re-prompt gate. `NeedSubject` = nothing typed
/// yet; `Mismatch` = typed text whose digest is NOT the stored hash (send
/// stays disabled, the row stays parked); `Ready` = verified, or a legacy
/// unsalted row with any non-empty retype (no hash to verify against).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DsarGate {
    NeedSubject,
    Mismatch,
    Ready,
}

pub fn dsar_gate(salted: bool, subject_hash: &str, typed: &str, salt: &str) -> DsarGate {
    let typed = typed.trim();
    if typed.is_empty() {
        return DsarGate::NeedSubject;
    }
    if !salted {
        return DsarGate::Ready;
    }
    if crate::queue::digest_salted(salt, typed) == subject_hash {
        DsarGate::Ready
    } else {
        DsarGate::Mismatch
    }
}

/// One parked row — destructive by construction, or parked at the retry cap
/// (v1.27.21 N5). `Replay` fires immediately (a manual, on-screen decision);
/// `Skip` drops it. A parked `Dsar` demands the subject be re-entered AND
/// hash-verified first (the hash is one-way; the retyped text must prove it).
#[component]
fn DestructiveRow(
    action: QueuedAction,
    on_done: EventHandler<QueuedAction>,
    on_skip: EventHandler<QueuedAction>,
) -> Element {
    let api = use_context::<Signal<ApiClient>>();
    let mut subject = use_signal(String::new);
    let busy = use_signal(|| false);
    let error = use_signal(|| None::<String>);

    let act = action.clone();
    let cause = act.clone();
    let skip_act = act.clone();

    // v1.27.21 (N2/N5): per-render row state — the DSAR gate and the
    // retry-exhausted note are derived, never stored.
    let gate = match &act {
        QueuedAction::Dsar {
            subject_hash,
            salted,
            ..
        } => {
            let salt = crate::queue::salt().read().clone().unwrap_or_default();
            dsar_gate(*salted, subject_hash, &subject(), &salt)
        }
        _ => DsarGate::Ready,
    };
    let gate_hint = match gate {
        DsarGate::NeedSubject => Some(crate::i18n::t("replay_subject_required")),
        DsarGate::Mismatch => Some(crate::i18n::t("replay_subject_mismatch")),
        DsarGate::Ready => None,
    };
    let retry_note = (act.retries() >= crate::queue::MAX_REPLAY_RETRIES).then(|| {
        crate::i18n::t_fmt("replay_retry_parked", &[act.retries().to_string()])
    });

    let cause_replay = move |_| {
        let api = api;
        let mut busy = busy;
        let action = cause.clone();
        let mut error = error;
        let subject = subject;
        let on_done = on_done;
        spawn(async move {
            busy.set(true);
            error.set(None);
            use QueuedAction::*;
            let res = match &action {
                // v1.27.21 (N8): the owner replays with the ids — it is the
                // erasure's scope, not optional commentary.
                Purge {
                    chunk_ids,
                    owner,
                    ..
                } => api().purge(chunk_ids, owner.as_deref()).await.map(|_| ()),
                Dsar {
                    subject_hash: _,
                    action: a,
                    ..
                } => {
                    let s = subject().trim().to_string();
                    if s.is_empty() {
                        error.set(Some(crate::i18n::t("replay_subject_required")));
                        busy.set(false);
                        return;
                    }
                    api().dsar(&s, a).await.map(|_| ())
                }
                // (N5): a retry-exhausted row replays by hand here — the same
                // calls the auto-replay loop makes.
                Approve {
                    id, supersedes, ..
                } => api()
                    .approve_proposal(*id, *supersedes, None)
                    .await
                    .map(|_| ()),
                Reject { id, .. } => api().reject_proposal(*id, None).await.map(|_| ()),
                Edit { id, content, .. } => {
                    api().edit_proposal(*id, content).await.map(|_| ())
                }
            };
            busy.set(false);
            if crate::queue::replay_applied(&res) {
                on_done.call(action);
            } else {
                error.set(Some(crate::api::error_message(res.as_ref().unwrap_err())));
            }
        });
    };

    let skip = move |_| on_skip.call(skip_act.clone());

    rsx! {
        li { class: "flex flex-col gap-1.5 rounded border border-border p-2",
            div { class: "flex items-center justify-between gap-2",
                span { class: "text-sm font-mono",
                    {crate::i18n::t(&format!("replay_kind_{}", act.kind()))}
                    " · " {summary(&act)}
                }
                span { class: "text-xs text-muted-foreground",
                    {crate::i18n::t("replay_queued_ago")} ""
                    {format!("{}m", queued_ago_minutes(action_queued_at(&act), crate::time_budget::now_unix()))}
                }
            }
            if let QueuedAction::Dsar { .. } = &act {
                input {
                    class: "input",
                    value: "{subject}",
                    oninput: move |e| subject.set(e.value()),
                    placeholder: crate::i18n::t("replay_subject_placeholder"),
                    "aria-label": crate::i18n::t("replay_subject_prompt"),
                }
            }
            if let Some(note) = &retry_note {
                p { class: "text-xs text-warn", role: "status", "{note}" }
            }
            div { class: "flex items-center gap-2",
                button {
                    class: "btn btn-destructive btn-sm",
                    disabled: busy() || gate != DsarGate::Ready,
                    onclick: cause_replay,
                    {crate::i18n::t("replay_replay")}
                }
                button {
                    class: "btn btn-outline btn-sm",
                    disabled: busy(),
                    onclick: skip,
                    {crate::i18n::t("replay_skip")}
                }
                if let Some(e) = error() {
                    span { class: "text-xs text-danger", "{e}" }
                }
                if let Some(hint) = gate_hint {
                    span { class: "text-xs text-warn", "{hint}" }
                }
            }
        }
    }
}

fn action_queued_at(action: &QueuedAction) -> i64 {
    match action {
        QueuedAction::Approve { queued_at, .. }
        | QueuedAction::Reject { queued_at, .. }
        | QueuedAction::Edit { queued_at, .. }
        | QueuedAction::Purge { queued_at, .. }
        | QueuedAction::Dsar { queued_at, .. } => *queued_at,
    }
}

/// v1.27.21 (N13) pure: has the local kept-set drifted from the parent's
/// `items`? Compared by idempotency key sequence, NOT length — a replay that
/// swaps one row for another (same count, different ids) must resync, which
/// the old length check silently missed.
pub fn kept_drifted(kept: &[QueuedAction], items: &[QueuedAction]) -> bool {
    let keys = |q: &[QueuedAction]| -> Vec<String> { q.iter().map(|a| a.key()).collect() };
    keys(kept) != keys(items)
}

/// The banner: every parked action (destructive or retry-exhausted), one per
/// row, each with its own Replay/Skip. Only `on_update` (the kept set)
/// mutates the queue.
#[component]
pub fn ReplayReview(
    items: Vec<QueuedAction>,
    on_update: EventHandler<Vec<QueuedAction>>,
) -> Element {
    let mut kept = use_signal(|| items.clone());
    // Prop → local sync: the banner re-mounts per queue change (the parent
    // re-renders with fresh `items`); fold edits into the kept set.
    use_effect(move || {
        if kept_drifted(&kept(), &items) {
            kept.set(items.clone());
        }
    });

    let dismiss_all = move |_| {
        kept.set(Vec::new());
        on_update.call(Vec::new());
    };

    rsx! {
        div { class: "card border-danger/40",
            div { class: "card-header",
                h2 { class: "card-title", {crate::i18n::t("replay_title")} }
                span { class: "text-sm text-muted-foreground", "{kept().len()}" }
            }
            div { class: "card-body space-y-2",
                p { class: "text-sm text-muted-foreground", {crate::i18n::t("replay_sub")} }
                ul { class: "space-y-2",
                    for a in kept() {
                        DestructiveRow {
                            key: "{a.key()}",
                            action: a.clone(),
                            on_done: {
                                let mut kept_done = kept;
                                let on_update_done = on_update;
                                move |a: QueuedAction| {
                                    let mut v = kept_done();
                                    v.retain(|x| x.key() != a.key());
                                    kept_done.set(v.clone());
                                    on_update_done.call(v);
                                }
                            },
                            on_skip: {
                                let mut kept_done = kept;
                                let on_update_done = on_update;
                                move |a: QueuedAction| {
                                    let mut v = kept_done();
                                    v.retain(|x| x.key() != a.key());
                                    kept_done.set(v.clone());
                                    on_update_done.call(v);
                                }
                            },
                        }
                    }
                }
                button {
                    class: "btn btn-ghost btn-sm",
                    onclick: dismiss_all,
                    {crate::i18n::t("replay_dismiss")}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// v1.27.21 tests — the parked-row pure cores (N2/N9/N13).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{digest_salted, MAX_REPLAY_RETRIES};

    fn dsar(subject: &str, salt: &str, salted: bool, retries: u8) -> QueuedAction {
        QueuedAction::Dsar {
            subject_hash: digest_salted(if salted { salt } else { "" }, subject),
            action: "both".into(),
            salted,
            queued_at: 1000,
            retries,
        }
    }

    /// N2: the retyped subject must hash to the stored digest before Replay
    /// arms; a legacy row needs only a non-empty retype; empty never arms.
    #[test]
    fn dsar_replay_requires_hash_match_except_legacy() {
        let salt = "per-install-salt";
        let hash = digest_salted(salt, "right@x");
        assert_eq!(
            dsar_gate(true, &hash, "right@x", salt),
            DsarGate::Ready,
            "the correct retype verifies"
        );
        assert_eq!(
            dsar_gate(true, &hash, "wrong@x", salt),
            DsarGate::Mismatch,
            "a mistyped subject never arms the send"
        );
        assert_eq!(
            dsar_gate(true, &hash, "", salt),
            DsarGate::NeedSubject,
            "empty retype never arms"
        );
        assert_eq!(
            dsar_gate(true, &hash, "  ", salt),
            DsarGate::NeedSubject,
            "whitespace-only is empty"
        );
        // A different install's salt cannot verify (the salt is per-install).
        assert_eq!(dsar_gate(true, &hash, "right@x", "other"), DsarGate::Mismatch);
        // Legacy unsalted rows: any non-empty retype is enough — there is no
        // verification form, and replay must never be permanently blocked.
        let legacy_hash = digest_salted("", "old@x");
        assert_eq!(dsar_gate(false, &legacy_hash, "anything@x", salt), DsarGate::Ready);
        assert_eq!(dsar_gate(false, &legacy_hash, "", salt), DsarGate::NeedSubject);
    }

    /// N9: a corrupt stored hash with multi-byte chars must truncate on char
    /// boundaries — the byte-slice cut panicked on exactly this input.
    #[test]
    fn summary_truncates_corrupt_hash_on_char_boundaries() {
        let row = dsar("s@x", "salt", true, 0);
        let sane = summary(&row);
        assert!(sane.starts_with("both subject "));
        assert!(sane.contains(&digest_salted("salt", "s@x")[..12]));
        // The panic case: byte 12 of this hash lands mid-char (1-byte 'a' +
        // 2-byte 'é's), so the old byte-slice cut panicked exactly here.
        let mut corrupt = row;
        if let QueuedAction::Dsar { subject_hash, .. } = &mut corrupt {
            *subject_hash = "aééééééé".to_string();
        }
        assert_eq!(
            summary(&corrupt),
            "both subject aééééééé…",
            "char-boundary truncation, never a panic"
        );
    }

    /// N13: prop sync compares key sequences, not lengths — same count with
    /// different rows is drift; a pure reorder is drift too (order is part of
    /// the review list).
    #[test]
    fn kept_drift_detects_same_length_row_swaps() {
        let a = QueuedAction::Reject {
            id: 1,
            queued_at: 5,
            retries: 0,
        };
        let b = QueuedAction::Reject {
            id: 2,
            queued_at: 6,
            retries: 0,
        };
        assert!(!kept_drifted(&[a.clone(), b.clone()], &[a.clone(), b.clone()]));
        // Same length, different row — the old length check missed this.
        assert!(kept_drifted(std::slice::from_ref(&a), std::slice::from_ref(&b)));
        // A retry bump alone is NOT drift (identity excludes volatile fields).
        let mut bumped = b.clone();
        if let QueuedAction::Reject { retries, .. } = &mut bumped {
            *retries = MAX_REPLAY_RETRIES;
        }
        assert!(!kept_drifted(&[bumped], &[b]));
    }
}
