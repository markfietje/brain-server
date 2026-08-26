//! v1.28.1 "Holdall" M4 (F-12 HIGH, F-35) — the shared two-step confirm for
//! destructive buttons. Every irreversible surface (purge, DSAR erasure,
//! quarantine delete, reindex) routes through ONE component so the discipline
//! is uniform — the command palette's destructive gate was the only confirm,
//! and two raw-click mutagen buttons (quarantine Delete, snapshots Reindex)
//! sat outside it.
//!
//! Interaction: click 1 arms (button becomes a warning row with a note +
//! Cancel + Confirm); click 2 films. `blocked` freezes the Confirm — the
//! purge panel requires its inline footprint preview, the subjects panel
//! requires a fresh preview for the CURRENT subject, so "erase" is always
//! one more deliberate step after "see".
//!
//! Pure core extracted so the component is plumbing: `arm_allowed` + the
//! existing-modifier tests pin the gate without a render harness.

use dioxus::prelude::*;

/// Pure: may the first click arm the confirm state? The button is inert
/// while disabled (conn/writes gate) — arming is impossible then.
pub fn arm_allowed(armed: bool, blocked: bool, disabled: bool) -> bool {
    !armed && !blocked && !disabled
}

/// Pure: may the Confirm button fire the irreversible action? Armed AND not
/// further blocked (stale preview, disabled writes).
pub fn confirm_allowed(armed: bool, blocked: bool, disabled: bool) -> bool {
    armed && !blocked && !disabled
}

/// The shared two-step confirm button. Renders the plain label until armed;
/// once armed it renders the note + Cancel + Confirm row, where Confirm is
/// frozen by `blocked`/`disabled` (a parent preview not yet rendered).
#[component]
pub fn ConfirmDestructive(
    label: String,
    note: String,
    blocked: bool,
    blocked_hint: Option<String>,
    disabled: bool,
    small: bool,
    on_confirm: EventHandler<()>,
    on_arm: Option<EventHandler<()>>,
) -> Element {
    let mut armed = use_signal(|| false);
    let cancel_lbl = crate::i18n::t("confirm_cancel");
    let arm_hint = blocked_hint.unwrap_or_else(|| note.clone());
    let size = if small { " btn-sm" } else { "" };

    let click = move |_| {
        if disabled {
            return;
        }
        armed.set(true);
        if let Some(arm) = on_arm {
            arm.call(());
        }
    };
    let cancel = move |_| armed.set(false);
    let confirm = move |_| {
        armed.set(false);
        on_confirm.call(());
    };

    rsx! {
        if armed() {
            div { class: "flex flex-wrap items-center gap-2 rounded border border-danger/40 p-2",
                span { class: "text-sm text-danger", "{arm_hint}" }
                span { class: "ms-auto flex gap-2",
                    button {
                        class: "btn btn-outline btn-sm",
                        onclick: cancel,
                        "{cancel_lbl}"
                    }
                    button {
                        class: "btn btn-destructive btn-sm",
                        disabled: !crate::confirm::confirm_allowed(armed(), blocked, disabled),
                        onclick: confirm,
                        "{label}"
                    }
                }
            }
        } else {
            button {
                class: "btn btn-destructive{size}",
                disabled: !crate::confirm::arm_allowed(armed(), blocked, disabled),
                onclick: click,
                "{label}"
            }
        }
    }
}
