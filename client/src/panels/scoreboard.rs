//! v1.28.20 Cockpit M3: the scoreboard panel — `GET /workflow/scoreboard`
//! rendered as a cards row (the Clients-panel dashboard card patterns) with
//! the weekly calibration badge. Read-only evidence: the endpoint is
//! Admin + DPO-role gated server-side; this panel is presentation only.

use crate::api::ApiClient;
use crate::i18n::{t, t_fmt};
use crate::panels::{PageTitle, use_document_title};
use crate::{Conn, UiState};
use dioxus::prelude::*;
use serde_json::Value;

/// The metric fields the endpoint ships, in render order — one source of
/// truth for the cards row (a new scorer field lands here or it doesn't
/// render; nothing is invented client-side).
pub const METRIC_FIELDS: &[&str] = &[
    "fcr_units",
    "repeat_contact_rate_units",
    "correctness_units",
    "override_rate_units",
    "gap_rate_units",
    "abstention_rate_units",
    "guidance_acceptance_units",
    "handoff_completeness_units",
    "escalation_honored_units",
];

/// Pure card model: (label key, value string) pairs + the two status flags.
/// Fail-safe over any shape: absent fields are skipped, never defaulted to 0.
pub fn scoreboard_cards(v: &Value) -> (Vec<(&'static str, String)>, bool, i64, bool) {
    let mut cards = Vec::new();
    for f in METRIC_FIELDS {
        if let Some(n) = v.get(*f).and_then(Value::as_i64) {
            cards.push((field_label(f), n.to_string()));
        }
    }
    (
        cards,
        v.get("audit_green")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        v.get("runs_scored").and_then(Value::as_i64).unwrap_or(0),
        v.get("calibration_report_emitted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

fn field_label(field: &str) -> &'static str {
    match field {
        "fcr_units" => "sb_fcr",
        "repeat_contact_rate_units" => "sb_repeat",
        "correctness_units" => "sb_correctness",
        "override_rate_units" => "sb_override",
        "gap_rate_units" => "sb_gap",
        "abstention_rate_units" => "sb_abstention",
        "guidance_acceptance_units" => "sb_guidance",
        "handoff_completeness_units" => "sb_handoff",
        "escalation_honored_units" => "sb_escalation",
        _ => "sb_fcr",
    }
}

#[component]
pub fn Scoreboard() -> Element {
    panel_scoreboard()
}

pub fn panel_scoreboard() -> Element {
    use_document_title(|| t("sb_title").to_string());
    let api = use_context::<Signal<ApiClient>>();
    let ui = use_context::<UiState>();
    let connected = (ui.conn)() == Conn::Connected;
    let res = use_resource(move || async move { api().workflow_scoreboard().await });
    rsx! {
        div { class: "space-y-3",
            PageTitle { {t("sb_title")} }
            if !connected {
                p { class: "text-sm text-muted-foreground", {t("connect_needed")} }
            }
            match &*res.read() {
                Some(Ok(v)) => {
                    let (cards, audit_green, runs, calibrated) = scoreboard_cards(v);
                    rsx! {
                        div { class: "flex gap-2 items-center flex-wrap",
                            span { class: if audit_green { "badge badge-ok" } else { "badge badge-warn" },
                                {t(if audit_green { "sb_audit_ok" } else { "sb_audit_notok" })}
                            }
                            span { class: "badge", {t_fmt("sb_runs", &[runs.to_string()])} }
                            if calibrated {
                                span { class: "badge badge-primary", {t("sb_calibrated")} }
                            }
                        }
                        div { class: "grid gap-3 sm:grid-cols-2 lg:grid-cols-3",
                            for (label, value) in &cards {
                                div { key: "{label}", class: "card p-4",
                                    p { class: "text-xs text-muted-foreground", {t(label)} }
                                    p { class: "text-2xl font-semibold tabular", "{value}" }
                                }
                            }
                        }
                    }
                }
                Some(Err(e)) => rsx! {
                    p { class: "text-sm text-danger shake", role: "alert", "{crate::api::error_message(e)}" }
                },
                None => rsx! {
                    p { class: "text-sm text-muted-foreground animate-pulse", {t("loading")} }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::conversation::{
        ComposerCommand, MAX_CRANK_STEPS, TimelineMarker, contradiction_pair, evidence_of,
        parse_command, timeline_marker,
    };

    #[test]
    fn scoreboard_panel_matches_route_shapes() {
        // The exact shipped wire shape renders every metric + both flags.
        let full = serde_json::json!({
            "fcr_units": 91, "repeat_contact_rate_units": 12,
            "correctness_units": 88, "override_rate_units": 3,
            "gap_rate_units": 5, "abstention_rate_units": 2,
            "guidance_acceptance_units": 70, "handoff_completeness_units": 95,
            "audit_green": true, "escalation_honored_units": 100,
            "runs_scored": 42, "calibration_report_emitted": true,
        });
        let (cards, green, runs, cal) = scoreboard_cards(&full);
        assert_eq!(cards.len(), 9, "all nine metrics render");
        assert_eq!(cards[0], ("sb_fcr", "91".to_string()));
        assert!(green && runs == 42 && cal);
        // Absent fields skip; a wrong-typed field never becomes a fake zero.
        let sparse = serde_json::json!({"fcr_units": 50});
        let (cards2, green2, runs2, _) = scoreboard_cards(&sparse);
        assert_eq!(cards2.len(), 1);
        assert!(
            !green2,
            "absent audit flag reads as not-green (fail-closed)"
        );
        assert_eq!(runs2, 0, "no runs scored is an honest zero");
        // A string where a number belongs renders nothing, never panics.
        let bad = serde_json::json!({"fcr_units": "many"});
        assert!(scoreboard_cards(&bad).0.is_empty());
    }

    #[test]
    fn parse_command_maps_the_cli_verbs() {
        assert_eq!(parse_command("/handoff"), Some(ComposerCommand::Handoff));
        assert_eq!(
            parse_command(" /scoreboard "),
            Some(ComposerCommand::Scoreboard)
        );
        assert_eq!(parse_command("/help"), Some(ComposerCommand::Help));
        assert_eq!(
            parse_command("/crank"),
            Some(ComposerCommand::Crank { steps: None })
        );
        assert_eq!(
            parse_command("/crank 25"),
            Some(ComposerCommand::Crank { steps: Some(25) })
        );
        // Bounded at the GUI cap; junk steps refuse.
        assert_eq!(
            parse_command("/crank 9999"),
            Some(ComposerCommand::Crank {
                steps: Some(MAX_CRANK_STEPS)
            })
        );
        assert_eq!(parse_command("/crank abc"), None);
        // Not a command → steering.
        assert_eq!(parse_command("focus on step 2"), None);
        assert_eq!(parse_command(""), None);
        assert_eq!(parse_command("/"), None);
        assert_eq!(parse_command("/unknown"), None);
        // Trailing args beyond the verb refuse (fail-closed parsing).
        assert_eq!(parse_command("/handoff extra"), None);
    }

    #[test]
    fn timeline_markers_classify_lineage_topics() {
        assert_eq!(
            timeline_marker("workflow/checkpoint"),
            TimelineMarker::Checkpoint
        );
        assert_eq!(timeline_marker("workflow/rewind"), TimelineMarker::Branch);
        assert_eq!(
            timeline_marker("workflow/ask_human"),
            TimelineMarker::AskHuman
        );
        assert_eq!(timeline_marker("workflow/log"), TimelineMarker::Plain);
        assert_eq!(timeline_marker(""), TimelineMarker::Plain);
    }

    #[test]
    fn evidence_viewer_extracts_and_links_pairs() {
        let out = serde_json::json!({
            "findings": [{"claim":"late escalation","origin":"frontdoor","confidence":0.8}],
            "contradictions": [["row says closed","log says open"]],
            "evidence": [{"digest":"abc"}],
            "questions": [{"question":"was SLA met?","justification":"timestamps","score_units":3}],
        });
        let ev = evidence_of(&serde_json::json!({"output": out}))
            .expect("structured evidence extracted");
        assert_eq!(ev["findings"].as_array().unwrap().len(), 1);
        // Contradictions render only as complete linked pairs.
        let pair = contradiction_pair(&ev["contradictions"][0]).expect("two-sided pair");
        assert_eq!(pair.0, "row says closed");
        assert_eq!(pair.1, "log says open");
        // A one-sided half refuses — a lone half would mislead.
        assert_eq!(contradiction_pair(&serde_json::json!(["only one"])), None);
        assert_eq!(contradiction_pair(&serde_json::json!("scalar")), None);
        // Object-shaped sides read their claim/text/id coordinate.
        let obj_pair = contradiction_pair(&serde_json::json!([
            {"claim":"a"},{"claim":"b"}
        ]))
        .expect("object sides link");
        assert_eq!(obj_pair, ("a".to_string(), "b".to_string()));
        // No structured evidence → no viewer (plain output path).
        assert_eq!(
            evidence_of(&serde_json::json!({"output": {"other": 1}})),
            None
        );
        assert_eq!(evidence_of(&serde_json::json!({"status": "settled"})), None);
    }
}
