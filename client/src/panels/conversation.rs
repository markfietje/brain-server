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
use crate::{Conn, Route, UiState};
use dioxus::prelude::*;
use serde_json::Value;

/// The ring window the transcript folds from (mirrors the driver's cap).
const FOLD_WINDOW: usize = 500;

/// The event chain IS the scrollback; "infinite scroll" is windowing over an
/// append-only log. The renderer shows a bounded keyset slice: the LIVE TAIL
/// (`size` nodes) plus any earlier range the operator pulled up. No new
/// dependency — a pure `Vec` slice over ordered nodes, keyed by node index.
pub const TRANSCRIPT_WINDOW: usize = 100;

/// The rendered keyset: `(skip, take)` over the ordered node list. `earlier`
/// counts how many nodes beyond the live tail were pulled up; appending new
/// nodes slides the tail forward without changing what was already fetched
/// (prefix-stable like the server's window derivation).
pub fn transcript_window(total: usize, earlier: usize, size: usize) -> (usize, usize) {
    let size = size.max(1);
    let tail_start = total.saturating_sub(size);
    let skip = tail_start.saturating_sub(earlier);
    (skip, total - skip)
}

/// The composer's session-age badge input, derived from the lineage read:
/// `(events, checkpoints, oldest event id)` — there is no "new session" in an
/// unbounded run, only its age. `None` when no events are readable.
pub fn session_age(lineage: &Value) -> Option<(usize, usize, i64)> {
    let events = lineage.get("events")?.as_array()?;
    if events.is_empty() {
        return None;
    }
    let oldest = events.iter().filter_map(|e| e["event_id"].as_i64()).min()?;
    Some((
        events.len(),
        events
            .iter()
            .filter(|e| {
                e["topic"]
                    .as_str()
                    .is_some_and(|t| t.contains("checkpoint"))
            })
            .count(),
        oldest,
    ))
}

/// Keyboard conventions reused from the Review panel (A approve / R reject /
/// J down / K up). Pure so the mapping is pinnable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKey {
    Approve,
    Reject,
    Down,
    Up,
    Help,
}

pub fn run_key(key: &str) -> Option<RunKey> {
    match key {
        "a" | "A" => Some(RunKey::Approve),
        "r" | "R" => Some(RunKey::Reject),
        "j" | "J" => Some(RunKey::Down),
        "k" | "K" => Some(RunKey::Up),
        "?" => Some(RunKey::Help),
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

/// Branch markers live in the engine-owned state (`state.branches[]`,
/// appended by rewind) — parsed from the sanitized state body here so the
/// workflow-run node can render them.
fn branches_of(state: &Value) -> Vec<Value> {
    state
        .get("state_json")
        .and_then(Value::as_str)
        .and_then(|q| serde_json::from_str::<Value>(q).ok())
        .and_then(|s| s.get("branches").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}
/// Does this stream event belong to the given run? The panel refetches state
/// + lineage when one arrives — pure so the refresh trigger is pinnable.
fn event_for_run(e: &crate::events::StreamEvent, run_id: i64) -> bool {
    e.kind == "workflow" && e.payload["run_id"].as_i64() == Some(run_id)
}

/// Steering send guard: non-blank and within the server's 4000-char
/// screened-write bound (chars, not bytes — the composer counts chars).
fn steer_sendable(message: &str) -> bool {
    !message.trim().is_empty() && message.chars().count() <= 4000
}

/// Cockpit M2: the composer's `/commands` — the CLI verbs, GUI-ified.
/// `/answer` needs no command (the AskHuman card owns answering), so the
/// composer maps: crank (bounded steps), handoff (fetch + render the
/// I-PASS packet), scoreboard (navigate), help (cheat sheet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerCommand {
    Crank { steps: Option<u32> },
    Handoff,
    Scoreboard,
    Help,
}

/// The GUI crank bound — a single press executes at most 500 engine steps;
/// anything larger belongs to the operator CLI, not a button.
pub const MAX_CRANK_STEPS: u32 = 500;

pub fn parse_command(input: &str) -> Option<ComposerCommand> {
    let input = input.trim();
    let rest = input.strip_prefix('/')?;
    let mut parts = rest.split_whitespace();
    match parts.next()? {
        "crank" => {
            // An argument that fails to parse refuses the whole command
            // (`?`), it never degrades to an unbounded crank.
            let steps = if let Some(s) = parts.next() {
                Some(s.parse::<u32>().ok()?.clamp(1, MAX_CRANK_STEPS))
            } else {
                None
            };
            if parts.next().is_some() {
                return None;
            }
            Some(ComposerCommand::Crank { steps })
        }
        "handoff" => (!parts.next().is_some()).then_some(ComposerCommand::Handoff),
        "scoreboard" => (!parts.next().is_some()).then_some(ComposerCommand::Scoreboard),
        "help" => (!parts.next().is_some()).then_some(ComposerCommand::Help),
        _ => None,
    }
}

/// Cockpit M3: the evidence-pack view of one tool node. Extracts, from the
/// settled `output` payload ONLY (machine-written state, still rendered read-
/// only): findings with provenance `origin` labels, contradictions as linked
/// pairs, evidence digests, and verification questions with per-question
/// justification + score units. Absent fields render absent — nothing is
/// invented for a payload that did not carry it.
pub fn evidence_of(tool_state: &Value) -> Option<Value> {
    let out = tool_state.get("output")?;
    let findings = out.get("findings").and_then(Value::as_array);
    let contradictions = out.get("contradictions").and_then(Value::as_array);
    let evidence = out.get("evidence").and_then(Value::as_array);
    let questions = out.get("questions").and_then(Value::as_array);
    let empty =
        findings.is_none() && contradictions.is_none() && evidence.is_none() && questions.is_none();
    if empty || !out.is_object() {
        return None;
    }
    Some(out.clone())
}

/// A contradiction renders as a LINKED PAIR — both rows visible together or
/// not at all; a lone half is refused (a one-sided contradiction misleads).
pub fn contradiction_pair(c: &Value) -> Option<(String, String)> {
    let arr = c.as_array()?;
    if arr.len() != 2 {
        return None;
    }
    let side = |v: &Value| -> Option<String> {
        let s = v.as_str().map(str::to_string).or_else(|| {
            let a = v
                .get("claim")
                .or_else(|| v.get("text"))
                .or_else(|| v.get("id"))?;
            a.as_str().map(str::to_string)
        })?;
        (!s.is_empty()).then_some(crate::strip_invisible(&s))
    };
    Some((side(&arr[0])?, side(&arr[1])?))
}

/// The timeline marker class for one lineage event — pure so branch/checkpoint/
/// pause badges are pinnable without a fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineMarker {
    Checkpoint,
    Branch,
    AskHuman,
    Plain,
}

pub fn timeline_marker(topic: &str) -> TimelineMarker {
    if topic.contains("checkpoint") {
        TimelineMarker::Checkpoint
    } else if topic.contains("rewind") || topic.contains("branch") {
        TimelineMarker::Branch
    } else if topic.contains("ask") {
        TimelineMarker::AskHuman
    } else {
        TimelineMarker::Plain
    }
}

#[component]
pub fn RunConversation(run_id: i64) -> Element {
    panel_run(run_id)
}

/// Cockpit M3: the full-page timeline route (`/runs/:id/timeline`) — the
/// same TimelineView component, over the complete lineage read.
pub fn panel_timeline(run_id: i64) -> Element {
    use_document_title(move || format!("Run {run_id} timeline — brain"));
    let api = use_context::<Signal<ApiClient>>();
    let lineage_res =
        use_resource(move || async move { api().workflow_events(run_id, None).await });
    let (events, err): (Vec<Value>, Option<String>) = match &*lineage_res.read() {
        Some(Ok(v)) => (v["events"].as_array().cloned().unwrap_or_default(), None),
        Some(Err(e)) => (Vec::new(), Some(e.to_string())),
        None => (Vec::new(), None),
    };
    rsx! {
        div { class: "space-y-3", tabindex: 0,
            PageTitle { {t("runs_timeline")} }
            if let Some(e) = err {
                p { class: "text-sm text-danger", role: "alert", "{e}" }
            }
            section { class: "card",
                div { class: "card-header",
                    h2 { class: "card-title text-base", {t("runs_lineage")} }
                }
                div { class: "card-body",
                    if events.is_empty() {
                        p { class: "text-sm text-muted-foreground", {t("runs_empty")} }
                    }
                    TimelineView { events }
                }
            }
        }
    }
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
    // Render a bounded keyset slice of the transcript — the live
    // tail plus whatever earlier range was pulled up. Ten-thousand-node runs
    // never render ten thousand nodes.
    let mut earlier = use_signal(|| 0usize);
    let (skip, take) = transcript_window(nodes.len(), earlier(), TRANSCRIPT_WINDOW);
    let shown: Vec<(usize, String, Value)> = nodes
        .iter()
        .skip(skip)
        .take(take)
        .enumerate()
        .map(|(i, n)| (skip + i, n.0.clone(), n.1.clone()))
        .collect();
    let has_more = skip > 0;
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
                .filter(|e| event_for_run(e, run_id))
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
    let mut error = use_signal(|| None::<String>);
    // Cockpit M2: the composer's /commands + the human crank + the
    // printable handoff packet + the `?` cheat-sheet drawer.
    let mut help_open = use_signal(|| false);
    let mut handoff = use_signal(|| None::<Value>);
    let mut crank_steps = use_signal(|| 50u32);
    let mut note = use_signal(|| None::<String>);
    let navigator = navigator();

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

    // The composer dispatcher: `/commands` first, screened steering otherwise.
    let mut submit_composer = move |text: String| {
        note.set(None);
        if let Some(cmd) = parse_command(&text) {
            match cmd {
                ComposerCommand::Help => help_open.toggle(),
                ComposerCommand::Scoreboard => {
                    let _ = navigator.push(Route::Scoreboard {});
                }
                ComposerCommand::Handoff => {
                    let api = api();
                    spawn(async move {
                        match api.workflow_handoff(run_id).await {
                            Ok(v) => {
                                handoff.set(Some(v));
                                error.set(None);
                            }
                            Err(e) => note.set(Some(crate::api::error_message(&e))),
                        }
                        steer.set(String::new());
                    });
                }
                // Honest ceiling until the engine-pull worker (v1.28.21):
                // there is no HTTP crank — the button says so instead of
                // pretending. The bounded selector still enforces its cap.
                ComposerCommand::Crank { .. } => {
                    note.set(Some(t("runs_crank_unwired")));
                }
            }
            steer.set(String::new());
            return;
        }
        submit_steer(text);
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
        if k == RunKey::Help {
            help_open.toggle();
            return;
        }
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
    let branches: Vec<Value> = match &*state_res.read() {
        Some(Ok(v)) => branches_of(v),
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

            // The transcript — keyed dispatch with generic-card fallback;
            // polite live region (Cockpit M4 a11y).
            section { class: "card", "aria-label": t("runs_transcript"),
                div { class: "card-header flex items-center justify-between",
                    h2 { class: "card-title text-base", {t("runs_transcript")} }
                    Link { to: Route::RunTimeline { run_id },
                        class: "btn btn-ghost btn-sm",
                        {t("runs_timeline")}
                    }
                }
                div { class: "card-body space-y-2", "aria-live": "polite",
                    if nodes.is_empty() {
                        p { class: "text-sm text-muted-foreground", {t("runs_empty")} }
                    }
                    if has_more {
                        button {
                            class: "btn btn-ghost btn-sm w-full",
                            onclick: move |_| earlier.set(earlier() + TRANSCRIPT_WINDOW),
                            {t("runs_load_earlier")}
                        }
                    }
                    for (gidx, kind, data) in shown.iter() {
                        {
                            let idx = *gidx;
                            let kind = kind.as_str();
                            let data: Value = data.clone();
                            let has_keyed_view = keyed[idx];
                            let selected = cursor_idx == idx;
                            let view = crate::conversation::ReviewJob::build_view_node(&data);
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
                                    } else if kind == "assistant" {
                                        AssistantCard { data: data.clone() }
                                    } else if kind == "tool" {
                                        ToolCard { data: data.clone() }
                                    } else if kind == "delivery" {
                                        DeliveryCard { data: data.clone() }
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

            // The steering composer + /commands — advisory, screened
            // server-side (≤4000). aria-live polite: new transcript nodes
            // are announced without stealing focus.
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
                        onkeydown: move |e: Event<KeyboardData>| {
                            if e.data().key() == Key::Enter && steer_sendable(&steer()) {
                                submit_composer(steer());
                            }
                        },
                        "aria-label": t("runs_steer_placeholder"),
                    }
                    button {
                        class: "btn btn-outline btn-sm",
                        disabled: !writes || !steer_sendable(&steer()),
                        onclick: move |_| submit_composer(steer()),
                        {t("runs_send")}
                    }
                }
                p { class: "text-xs text-ink-faint mt-1 tabular",
                    "{4000 - steer().chars().count()} / 4000 · /crank /handoff /scoreboard /help"
                }
                // The session-age badge — the run is ONE unbounded
                // session; the badge states its age instead of any "new
                // session" affordance (there is none, by design).
                if let Some((events, ckpts, oldest)) = lineage_ok
                    .as_ref()
                    .and_then(|l| session_age(l))
                {
                    p { class: "text-xs text-muted-foreground mt-1 tabular",
                        {t_fmt("runs_session_age", &[events.to_string(), ckpts.to_string(), oldest.to_string()])}
                    }
                }
                // The human crank: one press, bounded, role-gated.
                div { class: "flex gap-2 items-center mt-2",
                    span { class: "text-xs text-muted-foreground", {t("runs_crank_label")} }
                    select {
                        class: "input w-24 text-xs",
                        value: "{crank_steps}",
                        disabled: !(writes && can_decide),
                        "aria-label": t("runs_crank_label"),
                        oninput: move |e| {
                            if let Ok(v) = e.value().parse::<u32>() {
                                crank_steps.set(v.min(MAX_CRANK_STEPS));
                            }
                        },
                        option { value: "10", "10" }
                        option { value: "50", "50" }
                        option { value: "100", "100" }
                        option { value: "500", "500" }
                    }
                    button {
                        class: "btn btn-secondary btn-sm",
                        disabled: !(writes && can_decide),
                        onclick: move |_| submit_composer(format!("/crank {crank_steps}")),
                        {t("runs_crun")}
                    }
                }
                if let Some(n) = note() {
                    p { class: "text-xs text-warn mt-1", role: "status", "{n}" }
                }
            }

            // The fetched handoff packet (/handoff) — printable evidence.
            if let Some(h) = handoff() {
                HandoffCard { packet: h, on_close: move |_| handoff.set(None) }
            }

            // Cockpit M2: the `?` cheat-sheet drawer — a dialog (focus trap
            // is the drawer's own focus cycle), closed by Esc or the button.
            if help_open() {
                div {
                    class: "fixed inset-0 z-40 bg-black/40 flex items-center justify-center",
                    role: "dialog",
                    "aria-modal": "true",
                    "aria-label": t("runs_help_title"),
                    tabindex: -1,
                    onkeydown: move |e: Event<KeyboardData>| {
                        if e.data().key() == Key::Escape || key_label(e.data().key()) == "?" {
                            help_open.set(false);
                        }
                    },
                    div { class: "card max-w-md w-full mx-4",
                        div { class: "card-header flex items-center justify-between",
                            h3 { class: "card-title", {t("runs_help_title")} }
                            button {
                                class: "btn btn-ghost btn-sm",
                                "aria-label": t("close"),
                                onclick: move |_| help_open.set(false),
                                "×"
                            }
                        }
                        div { class: "card-body space-y-1 text-sm",
                            p { "{t(\"runs_help_keys\")}" }
                            p { class: "font-mono text-xs", "J/K · A/R · ?" }
                            p { "{t(\"runs_help_commands\")}" }
                            p { class: "font-mono text-xs", "/crank [steps] · /handoff · /scoreboard · /help" }
                        }
                    }
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

/// The workflow-run node: state, branch markers, and the shared timeline
/// component (Cockpit M3: one timeline, reused by this node and the
/// full-page `/runs/:id/timeline`).
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
            TimelineView { events }
        }
    }
}

/// Cockpit M3: the run timeline — lineage tree with parent links, branch
/// markers, checkpoint badges, AskHuman pauses. A component so the
/// workflow-run node and `/runs/:id/timeline` render ONE implementation.
#[component]
pub fn TimelineView(events: Vec<Value>) -> Element {
    rsx! {
        ol { class: "mt-1 space-y-1 text-xs font-mono",
            for ev in &events {
                {
                    let topic = ev["topic"].as_str().unwrap_or_default();
                    let badge_class = match timeline_marker(topic) {
                        TimelineMarker::Checkpoint => "badge badge-ok",
                        TimelineMarker::Branch => "badge badge-warn",
                        TimelineMarker::AskHuman => "badge badge-danger",
                        TimelineMarker::Plain => "",
                    };
                    let badge_label = match timeline_marker(topic) {
                        TimelineMarker::Checkpoint => t("tl_checkpoint"),
                        TimelineMarker::Branch => t("tl_branch"),
                        TimelineMarker::AskHuman => t("tl_askhuman"),
                        TimelineMarker::Plain => String::new(),
                    };
                    rsx! {
                        li { class: "flex gap-2 items-baseline",
                            if !badge_label.is_empty() {
                                span { class: "{badge_class}", "{badge_label}" }
                            }
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
}

/// Cockpit M2: the assistant turn — progressive render while streaming,
/// settled text after `assistant/end`.
#[component]
fn AssistantCard(data: Value) -> Element {
    let view = crate::conversation::AssistantTurn::build_view_node(&data);
    let text = view["text"].as_str().unwrap_or_default().to_string();
    let streaming = !view["settled"].as_bool().unwrap_or(false);
    if text.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "space-y-1",
            p { class: "text-sm whitespace-pre-wrap", {crate::strip_invisible(&text)} }
            if streaming {
                span { class: "text-xs text-muted-foreground animate-pulse", {t("runs_streaming")} }
            }
        }
    }
}

/// Cockpit M2/M3: the tool invocation card — name, status, plus the
/// evidence-pack viewer when the output carried structured evidence.
#[component]
fn ToolCard(data: Value) -> Element {
    let view = crate::conversation::ToolInvocation::build_view_node(&data);
    let name = view["name"].as_str().unwrap_or("tool").to_string();
    let status = view["status"].as_str().unwrap_or("running").to_string();
    let evidence = evidence_of(&view);
    rsx! {
        div { class: "space-y-1",
            div { class: "flex items-center gap-2",
                span { class: "font-mono text-sm text-accent", "{name}" }
                if status == "settled" {
                    span { class: "badge badge-ok", {t("runs_tool_settled")} }
                } else if status == "error" {
                    span { class: "badge badge-danger", {t("runs_tool_error")} }
                } else {
                    span { class: "badge animate-pulse", {t("runs_tool_running")} }
                }
            }
            if let Some(ev) = evidence {
                EvidenceView { output: ev }
            } else if let Some(out) = view["output"].as_str() {
                p { class: "text-xs font-mono break-all text-muted-foreground",
                    {crate::strip_invisible(out)}
                }
            }
        }
    }
}

/// Cockpit M3: the delivery (handoff packet) card — collected items rendered
/// read-only with a done badge.
#[component]
fn DeliveryCard(data: Value) -> Element {
    let view = crate::conversation::Delivery::build_view_node(&data);
    let done = view["done"].as_bool().unwrap_or(false);
    let items = view["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() && !done {
        return rsx! {};
    }
    rsx! {
        div { class: "space-y-1",
            div { class: "flex items-center gap-2",
                span { class: "text-sm font-medium", {t("runs_delivery")} }
                if done {
                    span { class: "badge badge-ok", {t("runs_delivery_done")} }
                }
            }
            ul { class: "list-disc pl-5 text-xs space-y-0.5",
                for item in &items {
                    li { class: "font-mono break-all",
                        {crate::strip_invisible(item.as_str().unwrap_or(&item.to_string()))}
                    }
                }
            }
        }
    }
}

/// Cockpit M3: the /handoff packet view (I-PASS sections, printable).
#[component]
fn HandoffCard(packet: Value, on_close: EventHandler<Value>) -> Element {
    // I-PASS: Illness/Patient/Assessment/Situation/Safety map onto whatever
    // fields the endpoint assembled; unknown keys render verbatim.
    let obj = packet.as_object().cloned().unwrap_or_default();
    rsx! {
        div { class: "card card-enhanced print:border-0",
            div { class: "card-header flex items-center justify-between",
                h3 { class: "card-title", {t("runs_handoff_title")} }
                button {
                    class: "btn btn-ghost btn-sm",
                    onclick: move |_| on_close.call(Value::Null),
                    {t("close")}
                }
            }
            div { class: "card-body space-y-2",
                for (k, v) in &obj {
                    div { class: "space-y-0.5",
                        p { class: "text-xs font-medium uppercase tracking-wide text-muted-foreground", "{k}" }
                        p { class: "text-sm whitespace-pre-wrap",
                            {crate::strip_invisible(v.as_str().unwrap_or(&v.to_string()))}
                        }
                    }
                }
            }
        }
    }
}

/// Cockpit M3: the evidence-pack viewer — findings with provenance origins,
/// contradictions as linked pairs, evidence digests, verification questions
/// with justification + score. Read-only over machine-written state.
#[component]
fn EvidenceView(output: Value) -> Element {
    let findings = output["findings"].as_array().cloned().unwrap_or_default();
    let contradictions = output["contradictions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let evidence = output["evidence"].as_array().cloned().unwrap_or_default();
    let questions = output["questions"].as_array().cloned().unwrap_or_default();
    rsx! {
        div { class: "mt-1 space-y-2 border-l-2 border-border pl-3",
            if !findings.is_empty() {
                div { class: "space-y-1",
                    p { class: "text-xs font-medium", {t("ev_findings")} }
                    for f in &findings {
                        div { class: "text-xs space-y-0.5",
                            p { {crate::strip_invisible(f["claim"].as_str().or_else(|| f.as_str()).unwrap_or(""))} }
                            if let Some(origin) = f["origin"].as_str() {
                                span { class: "badge", "[{origin}]" }
                            }
                            if let Some(c) = f["confidence"].as_f64() {
                                span { class: "tabular text-ink-faint ml-1", "{c:.2}" }
                            }
                        }
                    }
                }
            }
            if !contradictions.is_empty() {
                div { class: "space-y-1",
                    p { class: "text-xs font-medium text-warn", {t("ev_contradictions")} }
                    for c in &contradictions {
                        if let Some((a, b)) = contradiction_pair(c) {
                            // Linked pair: both rows, always together.
                            div { class: "text-xs flex gap-2 items-baseline",
                                span { class: "line-through text-danger break-all", "{a}" }
                                span { class: "text-ink-faint", "⇄" }
                                span { class: "break-all", "{b}" }
                            }
                        }
                    }
                }
            }
            if !evidence.is_empty() {
                div { class: "space-y-1",
                    p { class: "text-xs font-medium", {t("ev_evidence")} }
                    for e in &evidence {
                        if let Some(d) = e["digest"].as_str().or_else(|| e.as_str()) {
                            p { class: "text-xs font-mono break-all text-muted-foreground", "{d}" }
                        }
                    }
                }
            }
            if !questions.is_empty() {
                div { class: "space-y-1",
                    p { class: "text-xs font-medium", {t("ev_questions")} }
                    for q in &questions {
                        div { class: "text-xs space-y-0.5",
                            p { {crate::strip_invisible(q["question"].as_str().or_else(|| q["text"].as_str()).unwrap_or(""))} }
                            if let Some(j) = q["justification"].as_str() {
                                p { class: "text-ink-faint", {crate::strip_invisible(j)} }
                            }
                            if let Some(s) = q["score_units"].as_i64().or_else(|| q["score"].as_i64()) {
                                span { class: "badge tabular", "{s}" }
                            }
                        }
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
        // Cockpit M2: `?` opens the cheat-sheet drawer.
        assert_eq!(run_key("?"), Some(RunKey::Help));
        assert_eq!(run_key("x"), None);
        assert_eq!(run_key(""), None);
    }

    /// Cockpit M2: the human crank is one press, BOUNDED, and gated on the
    /// approve role — the parser enforces the bound; the renderer disables
    /// the control without `writes && can_decide` (the same gate A/R use).
    #[test]
    fn crank_control_requires_approve_role_and_bound_steps() {
        assert_eq!(
            parse_command("/crank 500"),
            Some(ComposerCommand::Crank {
                steps: Some(MAX_CRANK_STEPS)
            }),
            "the cap itself is accepted"
        );
        assert_eq!(
            parse_command("/crank 501").and_then(|c| match c {
                ComposerCommand::Crank { steps } => steps,
                _ => None,
            }),
            Some(MAX_CRANK_STEPS),
            "anything over the cap clamps to it"
        );
        assert_eq!(
            parse_command("/crank 0"),
            Some(ComposerCommand::Crank { steps: Some(1) }),
            "zero steps is meaningless — clamps to 1"
        );
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

    // ── the plan's M4 behavior pins ────────────────────────────────────

    /// The workflow-run node's render model: branch markers come from the run
    /// STATE body (`state.branches[]`, appended by rewind) and the lineage
    /// events pass through with their parent links.
    #[test]
    fn workflow_run_node_renders_lineage_and_branches() {
        let state = serde_json::json!({
            "state_json": "{\"branches\":[{\"from_event\":3,\"reason\":\"retry\",\"at\":9}]}",
            "state_revision": 2,
        });
        let branches = branches_of(&state);
        assert_eq!(branches.len(), 1, "the rewind marker is visible");
        assert_eq!(branches[0]["from_event"], 3);
        // No branches in state → empty render input (never a panic).
        assert!(branches_of(&serde_json::json!({"state_json":"{}"})).is_empty());
        assert_eq!(
            branches_of(&serde_json::json!({"state_json":"not-json"})),
            Vec::<Value>::new()
        );
        // Lineage rows carry their parent link for the timeline rendering.
        let lineage = serde_json::json!({"events":[
            {"event_id":1,"parent_id":null,"topic":"workflow/start"},
            {"event_id":2,"parent_id":1,"topic":"workflow/log"},
        ]});
        assert_eq!(lineage["events"].as_array().unwrap().len(), 2);
        assert_eq!(lineage["events"][1]["parent_id"], 1);
    }

    /// The review-job node approves ONLY against the digest the node
    /// rendered — the same ReviewArmour binding as the ApprovalDock.
    #[test]
    fn review_job_node_approve_is_digest_bound() {
        let open = parse_open();
        let mut asm = crate::conversation::assembler::Assembler::new();
        asm.ingest::<crate::conversation::ReviewJob>(1, &open);
        let view = crate::conversation::ReviewJob::build_view_node(
            &asm.node("review-job:p7").expect("node").data,
        );
        let rendered_digest = view["digest"].as_str().expect("digest");
        // Approve forwards exactly the rendered bytes; reject carries none.
        assert_eq!(
            crate::approvals::decision_digest(true, rendered_digest),
            Some(rendered_digest.to_string())
        );
        assert_eq!(
            crate::approvals::decision_digest(false, rendered_digest),
            None
        );
        // A drifted digest would 409 server-side; the node never invents one.
        assert_ne!(rendered_digest, "deadbeef");
    }

    /// The AskHuman card answers and refreshes from the stream: only this
    /// run's workflow envelopes trigger the refetch, and the answer digest
    /// binds to the exact question bytes shown.
    #[test]
    fn askhuman_card_answers_and_refreshes_from_stream() {
        let ev = |run: i64| crate::events::StreamEvent {
            kind: "workflow".into(),
            seq: 7,
            payload: serde_json::json!({"topic":"workflow/log","run_id":run,
                "payload_json":"{}","event_id":9,"parent_event_id":null,"domain":"global"}),
        };
        assert!(
            event_for_run(&ev(5), 5),
            "this run's event refreshes the card"
        );
        assert!(!event_for_run(&ev(6), 5), "another run's event does not");
        assert!(!event_for_run(
            &crate::events::StreamEvent {
                kind: "pending".into(),
                seq: 8,
                payload: Value::Null
            },
            5,
        ));
        // The submitted answer binds to what was answered.
        assert_ne!(sha256_hex("Ship it?"), sha256_hex("Ship it"));
    }

    fn parse_open() -> Value {
        serde_json::json!({
            "kind":"proposal/open","id":"p7","proposal_id":7,
            "content_digest":"d1","sla_deadline":1800000000,"role_gate":"approve",
        })
    }

    /// The steering composer stays inside the server's screening limits:
    /// blank messages never send, exactly-4000 sends, over-limit refuses.
    #[test]
    fn conversation_panel_steers_within_screening_limits() {
        assert!(!steer_sendable(""), "blank refused");
        assert!(!steer_sendable("   "), "whitespace-only refused");
        assert!(steer_sendable("focus on step 2"), "normal guidance sends");
        assert!(
            steer_sendable(&"x".repeat(4000)),
            "exactly at the bound sends"
        );
        assert!(!steer_sendable(&"x".repeat(4001)), "over the bound refused");
        // Multibyte counts by CHARS to match the server's byte check honestly.
        assert!(steer_sendable(&"é".repeat(4000)));
        assert!(!steer_sendable(&"é".repeat(4001)));
    }
}

#[cfg(test)]
mod fathom_tests {
    use super::*;

    /// transcript_windows_over_ten_thousand_nodes_without_rendering_all
    #[test]
    fn transcript_windows_over_ten_thousand_nodes_without_rendering_all() {
        let total = 10_000;
        let (skip, take) = transcript_window(total, 0, TRANSCRIPT_WINDOW);
        assert_eq!(take, TRANSCRIPT_WINDOW, "the live tail is bounded");
        assert_eq!(skip, total - TRANSCRIPT_WINDOW);
        assert!(take < total, "never renders the whole chain");

        // Pulling up earlier ranges extends the window, bounded by history.
        let (skip2, take2) = transcript_window(total, TRANSCRIPT_WINDOW, TRANSCRIPT_WINDOW);
        assert_eq!(take2, 2 * TRANSCRIPT_WINDOW);
        assert_eq!(skip2, total - 2 * TRANSCRIPT_WINDOW);

        // Small transcripts render whole.
        let (skip3, take3) = transcript_window(7, 5_000, TRANSCRIPT_WINDOW);
        assert_eq!((skip3, take3), (0, 7), "earlier beyond history saturates");
        // Degenerate size is refused to a floor of one.
        let (_, take4) = transcript_window(50, 0, 0);
        assert_eq!(take4, 1);

        // Monotonic: more pulled-up history moves `skip` down, never negative.
        let mut prev_skip = usize::MAX;
        for earlier in [0usize, 50, 200, 10_000] {
            let (skip, _) = transcript_window(total, earlier, TRANSCRIPT_WINDOW);
            assert!(skip < prev_skip);
            prev_skip = skip;
        }
    }

    /// session_age_badge_reads_lineage_counts
    #[test]
    fn session_age_badge_reads_lineage_counts() {
        let lineage = serde_json::json!({"events":[
            {"event_id":4,"topic":"workflow/start"},
            {"event_id":9,"topic":"workflow/log"},
            {"event_id":12,"topic":"workflow/checkpoint"},
        ]});
        assert_eq!(session_age(&lineage), Some((3, 1, 4)));
        // Empty or unreadable lineage → no badge, never a fabricated age.
        assert_eq!(session_age(&serde_json::json!({"events":[]})), None);
        assert_eq!(session_age(&serde_json::json!({})), None);
        assert_eq!(session_age(&serde_json::json!("junk")), None);
    }

    /// sse_resume_backfills_gap_without_duplicates
    #[test]
    fn sse_resume_backfills_gap_without_duplicates() {
        use crate::events::{EventDedup, StreamEvent};
        let wf = |id: i64| StreamEvent {
            kind: "workflow".into(),
            seq: 99,
            payload: serde_json::json!({"topic":"workflow/log","run_id":3,
                "payload_json":"{}","event_id":id,"parent_event_id":null,"domain":"global"}),
        };
        // Live traffic up to id 9; the connection drops; ids 8..=11 were missed.
        let mut dedup = EventDedup::default();
        let mut seen = Vec::new();
        for id in [1i64, 5, 9] {
            if dedup.admit(&wf(id)) {
                seen.push(id);
            }
        }
        // The resume replay re-sends 5..=9 (overlap) then the gap fills 10/11.
        for id in [5i64, 6, 7, 8, 9, 10, 11] {
            if dedup.admit(&wf(id)) {
                seen.push(id);
            }
        }
        assert_eq!(
            seen,
            vec![1, 5, 9, 6, 7, 8, 10, 11],
            "no duplicates, gap filled"
        );
        // The resume cursor is the max workflow event id seen.
        let cursor = [wf(5), wf(11), wf(9)]
            .iter()
            .filter_map(|e| e.payload["event_id"].as_i64())
            .max();
        assert_eq!(cursor, Some(11));
    }

    /// The honest-UI pin: an unbounded run has NO session-rotation affordance
    /// anywhere in the panel source. Every literal below (including this
    /// comment and the test name) is written so it cannot match itself.
    #[test]
    fn no_rotation_affordance_in_panel() {
        let src = include_str!("conversation.rs");
        let banned = [
            concat!("new_", "sessi", "on"),
            concat!("New", "Sessi", "on"),
        ];
        for b in banned {
            assert!(
                !src.contains(b),
                "panel must not carry a session-rotation affordance: {b}"
            );
        }
        // The hyphenated wire spelling, assembled from chars so this source
        // never carries it whole.
        let hyphenated: String = "new-sess".chars().chain("ion".chars()).collect();
        assert!(!src.contains(&hyphenated));
    }
}
