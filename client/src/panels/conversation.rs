//! v1.28.19 Witness: the `/runs/:run_id` conversation surface — the smallest
//! transcript that makes the chat nodes real (the full chat console is
//! Cockpit's). The run's stream events fold through the conversation
//! assembler; the keyed `conversation.chat.node` dispatch renders the two
//! populated node kinds (`review-job`, `workflow-run`) with a generic-card
//! fallback for everything else. The AskHuman card is the GUI's answer loop;
//! the composer posts screened steering. Server stays authoritative — this
//! panel only changes where the human stands.

use crate::api::ApiClient;
use crate::conversation::assembler::Assembler;
use crate::i18n::{t, t_fmt};
use crate::panels::{PageTitle, use_document_title};
use crate::{Conn, UiState};
use dioxus::prelude::*;
use serde_json::Value;

/// The ring window the transcript folds from (mirrors the driver's cap).
const FOLD_WINDOW: usize = 500;

/// Keyboard conventions reused from the Review panel (A approve / R reject /
/// J down / K up). Pure so the mapping is pinnable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKey {
    Approve,
    Reject,
    Down,
    Up,
}

pub fn run_key(key: &str) -> Option<RunKey> {
    match key {
        "a" | "A" => Some(RunKey::Approve),
        "r" | "R" => Some(RunKey::Reject),
        "j" | "J" => Some(RunKey::Down),
        "k" | "K" => Some(RunKey::Up),
        _ => None,
    }
}

/// `KeyboardEvent.key` → the letter label (the review.rs convention).
fn key_label(key: Key) -> String {
    match key {
        Key::Character(c) => c.as_str().to_string(),
        other => format!("{other:?}"),
    }
}

/// SHA-256 hex of the exact question bytes — the answer binds to what was
/// answered (the server verifies against the live `pending_question`).
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Extract the AskHuman card fields from a sanitized state body.
/// `{pending_question, state_revision}` present → an answer is owed.
fn askhuman_of(state: &Value) -> Option<(String, i64)> {
    let q = state.get("state_json").and_then(Value::as_str)?;
    let s: Value = serde_json::from_str(q).ok()?;
    let question = s.get("pending_question")?.as_str()?.to_string();
    let rev = state.get("state_revision").and_then(Value::as_i64)?;
    if question.is_empty() {
        return None;
    }
    Some((question, rev))
}

#[component]
pub fn RunConversation(run_id: i64) -> Element {
    panel_run(run_id)
}

/// The transcript body (plain fn — the router-facing `RunConversation`
/// component wraps it).
pub fn panel_run(run_id: i64) -> Element {
    use_document_title(move || format!("Run {run_id} — brain"));
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let host = use_context::<Signal<crate::plugins::PluginHost>>();
    let connected = (ui.conn)() == Conn::Connected;
    let writes = (ui.writes_enabled)();
    let roles = api().roles();
    let can_decide = crate::role::role_allows(&roles, "approve");

    // The transcript: fold every stream event through the assembler. The
    // fold is deterministic over the event list, so re-folding the whole
    // bounded window on change is correct and simplest (O(window) per event,
    // window ≤ 500).
    let mut asm = use_signal(Assembler::new);
    use_effect(move || {
        let events = ui.events.read();
        let tail = events.len().saturating_sub(FOLD_WINDOW);
        let mut fresh = Assembler::new();
        for e in events.iter().skip(tail) {
            e.ingest(&mut fresh);
        }
        asm.set(fresh);
    });

    // Owned render model (the assembler read guard cannot cross into rsx):
    // ordered (kind, view-data) pairs plus the keyed-dispatch verdicts.
    let nodes: Vec<(String, Value)> = asm
        .read()
        .snapshot()
        .iter()
        .map(|n| (n.kind.to_string(), n.data.clone()))
        .collect();
    let max_cursor = nodes.len().saturating_sub(1);
    let keyed: Vec<bool> = {
        let host_read = host.read();
        nodes
            .iter()
            .map(|(kind, _)| crate::ui_renderer::chat_node_view(&host_read.slots, kind).is_some())
            .collect()
    };
    let mut cursor = use_signal(|| 0usize);
    if cursor() > max_cursor {
        cursor.set(max_cursor);
    }
    let cursor_idx = cursor();

    // The run state + lineage, refreshed by any workflow event on THIS run.
    let bump = use_signal(|| 0u32);
    {
        let mut b = bump;
        use_effect(move || {
            let mine = ui
                .events
                .read()
                .iter()
                .filter(|e| e.kind == "workflow" && e.payload["run_id"].as_i64() == Some(run_id))
                .cloned()
                .collect::<Vec<_>>();
            if !mine.is_empty() {
                b += 1;
            }
        });
    }
    let state_res = use_resource(move || {
        let api = api();
        let _ = bump();
        async move { api.workflow_state(run_id).await }
    });
    let lineage_res = use_resource(move || {
        let api = api();
        let _ = bump();
        async move { api.workflow_events(run_id, None).await }
    });

    let mut answer = use_signal(String::new);
    let mut steer = use_signal(String::new);
    let error = use_signal(|| None::<String>);

    let submit_answer = move |question: String| {
        let api = api();
        let mut answer = answer;
        let mut error = error;
        let mut b = bump;
        spawn(async move {
            let digest = sha256_hex(&question);
            let res = api.workflow_answer(run_id, &answer(), &digest).await;
            if let Err(e) = res {
                error.set(Some(crate::api::error_message(&e)));
            } else {
                answer.set(String::new());
                error.set(None);
                b += 1;
            }
        });
    };

    let submit_steer = move |message: String| {
        let api = api();
        let mut steer = steer;
        let mut error = error;
        spawn(async move {
            if message.trim().is_empty() {
                return;
            }
            let res = api.workflow_steer(run_id, &message).await;
            if let Err(e) = res {
                error.set(Some(crate::api::error_message(&e)));
            } else {
                steer.set(String::new());
                error.set(None);
            }
        });
    };

    // Digest-bound decision from INSIDE a review-job node — the ApprovalDock's
    // action moved to where the evidence streams in (approvals.rs law).
    let decide = move |approve: bool, proposal_id: i64, digest: Option<String>| {
        let api = api();
        let mut error = error;
        spawn(async move {
            let res = if approve {
                let bound = crate::approvals::decision_digest(true, &digest.unwrap_or_default());
                api.approve_proposal(proposal_id, None, bound.as_deref())
                    .await
                    .map(|_| ())
            } else {
                api.reject_proposal(proposal_id, None).await.map(|_| ())
            };
            if let Err(e) = res {
                error.set(Some(crate::api::error_message(&e)));
            }
        });
    };

    // Keyboard: same letters as Review. A/R act on the cursor node when it is
    // an undecided review-job; j/k walk the transcript. The node list rides
    // as an owned snapshot (cloned per event) — the closure must be 'static.
    let nodes_for_keys = nodes.clone();
    let onkeydown = move |e: Event<KeyboardData>| {
        let key = key_label(e.data().key());
        let Some(k) = run_key(&key) else {
            return;
        };
        let idx = cursor().min(max_cursor);
        if let Some((kind, data)) = nodes_for_keys.get(idx)
            && kind == "review-job"
            && k != RunKey::Down
            && k != RunKey::Up
        {
            let view = crate::conversation::ReviewJob::build_view_node(data);
            let pid = view["proposal_id"].as_i64();
            let terminal = view["terminal"].as_bool().unwrap_or(true);
            if !terminal
                && let Some(pid) = pid
                && can_decide
                && writes
            {
                let digest = view["digest"].as_str().unwrap_or_default().to_string();
                decide(k == RunKey::Approve, pid, Some(digest));
            }
        } else if k == RunKey::Down {
            cursor.set((idx + 1).min(max_cursor));
        } else if k == RunKey::Up {
            cursor.set(idx.saturating_sub(1));
        }
    };

    let askhuman: Option<(String, i64)> = match &*state_res.read() {
        Some(Ok(v)) => askhuman_of(v),
        _ => None,
    };
    let branches: Vec<Value> = match &*lineage_res.read() {
        Some(Ok(v)) => v
            .get("branches")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let (lineage_ok, lineage_err): (Option<Value>, Option<String>) = match &*lineage_res.read() {
        Some(Ok(v)) => (Some(v.clone()), None),
        Some(Err(e)) => (None, Some(e.to_string())),
        None => (None, None),
    };

    rsx! {
        div { class: "space-y-3", tabindex: 0, onkeydown,
            PageTitle { {t_fmt("runs_title", &[run_id.to_string()])} }
            if !connected {
                p { class: "text-sm text-muted-foreground", {t("connect_needed")} }
            }
            if let Some(e) = error() {
                p { class: "text-sm text-danger shake", role: "alert", "{e}" }
            }

            // THE live AskHuman card — pending_question + the answer input.
            if let Some((question, _rev)) = askhuman.clone() {
                div { class: "card card-enhanced border-warn",
                    role: "region", "aria-label": t("runs_askhuman"),
                    div { class: "card-header flex items-center gap-2",
                        span { class: "badge badge-warn", {t("runs_askhuman")} }
                    }
                    div { class: "card-body space-y-2",
                        p { class: "whitespace-pre-wrap text-sm",
                            {crate::strip_invisible(&question)}
                        }
                        div { class: "flex gap-2",
                            input {
                                class: "input flex-1",
                                placeholder: t("runs_answer_placeholder"),
                                value: "{answer}",
                                disabled: !(writes && can_decide),
                                oninput: move |e| answer.set(e.value()),
                                "aria-label": t("runs_answer_placeholder"),
                            }
                            button {
                                class: "btn btn-primary btn-sm",
                                disabled: !(writes && can_decide) || answer().trim().is_empty(),
                                onclick: move |_| submit_answer(question.clone()),
                                {t("runs_submit")}
                            }
                        }
                    }
                }
            }

            // The transcript — keyed dispatch with generic-card fallback.
            section { class: "card", "aria-label": t("runs_transcript"),
                div { class: "card-header",
                    h2 { class: "card-title text-base", {t("runs_transcript")} }
                }
                div { class: "card-body space-y-2",
                    if nodes.is_empty() {
                        p { class: "text-sm text-muted-foreground", {t("runs_empty")} }
                    }
                    for (idx, (kind, data)) in nodes.iter().enumerate() {
                        {
                            let has_keyed_view = keyed[idx];
                            let selected = cursor_idx == idx;
                            let view = crate::conversation::ReviewJob::build_view_node(data);
                            let node_key = kind.to_string();
                            rsx! {
                                div {
                                    key: "{node_key}-{idx}",
                                    class: if selected { "rounded-lg border border-accent p-3" } else { "rounded-lg border border-border p-3" },
                                    onclick: move |_| cursor.set(idx),
                                    if !has_keyed_view {
                                        // Generic-card fallback: never silently dropped.
                                        p { class: "text-sm font-mono text-muted-foreground",
                                            "[{node_key}]"
                                        }
                                    } else if kind == "review-job" && !view["fallback"].as_bool().unwrap_or(false) {
                                        ReviewJobCard { view, can_decide: writes && can_decide,
                                            on_decide: move |d: (bool, i64, Option<String>)| decide(d.0, d.1, d.2) }
                                    } else if kind == "workflow-run" {
                                        WorkflowRunCard { data: data.clone(),
                                            lineage_ok: lineage_ok.clone(),
                                            lineage_err: lineage_err.clone(),
                                            branches: branches.clone() }
                                    } else {
                                        p { class: "text-sm font-mono text-muted-foreground",
                                            "[{node_key}]"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // The steering composer — advisory, screened server-side (≤4000).
            div { class: "card p-3",
                label { class: "text-xs text-muted-foreground", {t("runs_steer")} }
                div { class: "flex gap-2 mt-1",
                    input {
                        class: "input flex-1",
                        maxlength: 4000,
                        placeholder: t("runs_steer_placeholder"),
                        value: "{steer}",
                        disabled: !writes,
                        oninput: move |e| steer.set(e.value()),
                        "aria-label": t("runs_steer_placeholder"),
                    }
                    button {
                        class: "btn btn-outline btn-sm",
                        disabled: !writes || steer().trim().is_empty(),
                        onclick: move |_| submit_steer(steer()),
                        {t("runs_send")}
                    }
                }
                p { class: "text-xs text-ink-faint mt-1 tabular",
                    "{4000 - steer().chars().count()} / 4000"
                }
            }
        }
    }
}

/// The review-job node: title, digest, SLA clock, role gate + inline
/// digest-bound approve/reject (moved here from the ApprovalDock; the dock
/// itself remains on Overview).
#[component]
fn ReviewJobCard(
    view: Value,
    can_decide: bool,
    on_decide: EventHandler<(bool, i64, Option<String>)>,
) -> Element {
    let pid = view["proposal_id"].as_i64().unwrap_or_default();
    let digest = view["digest"].as_str().unwrap_or("").to_string();
    let terminal = view["terminal"].as_bool().unwrap_or(false);
    let gate = view["role_gate"].as_str().unwrap_or("approve");
    let sla_line = view["sla_deadline"]
        .as_i64()
        .filter(|_| !terminal)
        .map(|deadline| {
            crate::time_budget::format_remaining(crate::time_budget::remaining(
                deadline,
                crate::time_budget::now_unix(),
            ))
        });
    rsx! {
        div { class: "space-y-1",
            div { class: "flex items-center justify-between gap-2",
                // i18n-exempt: wire coordinates rendered verbatim.
                span { class: "font-mono text-sm text-accent", "proposal #{pid}" }
                if terminal {
                    if view["approved"].as_bool().unwrap_or(false) {
                        span { class: "badge badge-ok", {t("approve")} }
                    } else {
                        span { class: "badge badge-danger", {t("reject")} }
                    }
                } else {
                    span { class: "badge", "role:{gate}" }
                }
            }
            if !digest.is_empty() {
                // i18n-exempt: content fingerprint, not prose.
                p { class: "text-xs text-ink-faint font-mono break-all", "{digest}" }
            }
            if let Some(ttl) = sla_line {
                p { class: "text-xs tabular",
                    {t_fmt("dock_sla", &[ttl])}
                }
            }
            if !terminal && can_decide {
                div { class: "flex gap-2 mt-1",
                    button {
                        class: "btn btn-primary btn-sm",
                        onclick: move |_| on_decide.call((true, pid, Some(digest.clone()))),
                        {t("approve")}
                    }
                    button {
                        class: "btn btn-outline btn-sm",
                        onclick: move |_| on_decide.call((false, pid, None)),
                        {t("reject")}
                    }
                }
            }
        }
    }
}

/// The workflow-run node: lineage timeline (events with parents + branch
/// markers), current decision state, and the last crank log line.
#[component]
fn WorkflowRunCard(
    data: Value,
    lineage_ok: Option<Value>,
    lineage_err: Option<String>,
    branches: Vec<Value>,
) -> Element {
    let events = lineage_ok
        .as_ref()
        .and_then(|v| v["events"].as_array().cloned())
        .unwrap_or_default();
    rsx! {
        div { class: "space-y-1",
            div { class: "flex items-center justify-between",
                span { class: "font-mono text-sm text-accent", "{data[\"state\"]}" }
            }
            if !branches.is_empty() {
                p { class: "text-xs text-warn",
                    {t_fmt("runs_branches", &[branches.len().to_string()])}
                }
            }
            if let Some(e) = lineage_err {
                p { class: "text-xs text-danger", "{e}" }
            }
            ol { class: "mt-1 space-y-1 text-xs font-mono",
                for ev in &events {
                    li { class: "flex gap-2 items-baseline",
                        if let Some(parent) = ev["parent_id"].as_i64() {
                            span { class: "text-ink-faint",
                                "└ {parent}→{ev[\"event_id\"]}"
                            }
                        } else {
                            span { class: "text-ink-faint", "• {ev[\"event_id\"]}" }
                        }
                        span { "{ev[\"topic\"]}" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_reuses_review_conventions() {
        assert_eq!(run_key("a"), Some(RunKey::Approve));
        assert_eq!(run_key("R"), Some(RunKey::Reject));
        assert_eq!(run_key("j"), Some(RunKey::Down));
        assert_eq!(run_key("k"), Some(RunKey::Up));
        assert_eq!(run_key("x"), None);
        assert_eq!(run_key(""), None);
    }

    #[test]
    fn askhuman_card_reads_pending_question_with_revision() {
        let state = serde_json::json!({
            "state_json": "{\"pending_question\":\"Ship it?\"}",
            "state_revision": 4,
        });
        assert_eq!(askhuman_of(&state), Some(("Ship it?".to_string(), 4)));
        // No question / empty question → no card (fail-safe rendering).
        let none = serde_json::json!({"state_json":"{}","state_revision":1});
        assert_eq!(askhuman_of(&none), None);
        let empty =
            serde_json::json!({"state_json":"{\"pending_question\":\"\"}","state_revision":1});
        assert_eq!(askhuman_of(&empty), None);
        // Corrupt body → no card, never a panic.
        let corrupt = serde_json::json!({"state_json":"not-json","state_revision":1});
        assert_eq!(askhuman_of(&corrupt), None);
    }

    #[test]
    fn answer_binds_the_question_it_saw() {
        // The digest is over the exact question bytes (server re-verifies).
        let d1 = sha256_hex("Ship it?");
        let d2 = sha256_hex("Ship it");
        assert_ne!(d1, d2, "a one-char drift changes the binding");
        assert_eq!(d1, sha256_hex("Ship it?"), "same bytes → same digest");
        assert_eq!(d1.len(), 64);
    }
}
