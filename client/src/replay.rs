//! v1.28.1 "Holdall" M4 (F-34) — the destructive replay review. On reconnect
//! the queue auto-replays approve/reject/edit ONLY; `Purge`/`Dsar` actions
//! park here, behind an explicit per-item decision (Replay / Skip / Dismiss
//! all). No irreversible op ever auto-fires.
//!
//! A parked `Dsar` persists only its subject's SHA-256 hash — replaying
//! RE-PROMPTS the subject (the hash is one-way; the reviewer retypes the
//! email in front of the pending erasure). A parked `Purge` replays with the
//! ids it carried (the queue never stored owner text).

use crate::api::ApiClient;
use crate::queue::QueuedAction;
use dioxus::prelude::*;

/// Pure: minutes since `queued_at` (ceil, floor at 0 → "just now").
pub fn queued_ago_minutes(queued_at: i64, now: i64) -> i64 {
    ((now - queued_at).max(0) + 59) / 60
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
        } => format!(
            "{a} subject {}…",
            &subject_hash[..subject_hash.len().min(12)]
        ),
        other => other.kind().to_string(),
    }
}

/// One destructive row. `Replay` fires immediately; `Skip` drops it. A parked
/// DSAR demands the subject be re-entered first (hash is one-way).
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
                Purge { chunk_ids, .. } => api().purge(chunk_ids, None).await.map(|_| ()),
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
                _ => Ok(()),
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
            div { class: "flex items-center gap-2",
                button {
                    class: "btn btn-destructive btn-sm",
                    disabled: busy(),
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

/// The banner: every parked destructive action, one per row, each with its
/// own Replay/Skip. Only `on_update` (the kept set) mutates the queue.
#[component]
pub fn ReplayReview(
    items: Vec<QueuedAction>,
    on_update: EventHandler<Vec<QueuedAction>>,
) -> Element {
    let mut kept = use_signal(|| items.clone());
    // Prop → local sync: the banner re-mounts per queue change (the parent
    // re-renders with fresh `items`); fold edits into the kept set.
    use_effect(move || {
        if kept().len() != items.len() {
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
