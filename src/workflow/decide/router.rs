//! The router — the pure port of the laya `router.py` contract
//! (LAYA_RUST_PORT §0.3/§2.2): checkpoint naming, the closed precedence
//! chain, the typed-decisions exact-match table, and the LRU state
//! machine. Pure: no loading happens here — the real `CheckpointLoader`
//! lives behind the Phase 1 feature; tests drive a stub.

use super::lang::Detection;
use std::collections::BTreeSet;

pub(crate) const BUNDLE_REPO: &str = "convaiinnovations/laya";
pub(crate) const CHECKPOINTS: &[&str] = &["english", "multilingual", "typed-decisions"];

/// The alias table: short names normalize onto the three checkpoints;
/// everything else is a named refusal (the source's ValueError).
pub(crate) fn normalise_name(name: &str) -> Result<String, String> {
    match name.trim().to_lowercase().as_str() {
        "english" | "en" | "laya" | "default" => Ok("english".into()),
        "multilingual" | "multi" | "ml" | "laya-multilingual" => Ok("multilingual".into()),
        "typed-decisions" | "typed" | "typed_decisions" | "decisions" => {
            Ok("typed-decisions".into())
        }
        other => Err(format!(
            "unknown checkpoint name: {other} — expected one of english | \
             multilingual | typed-decisions"
        )),
    }
}

/// The four fine-tuned typed-decisions workflows, matched by EXACT id-set
/// equality only — partial, superset, and empty sets match nothing (the
/// presets must never auto-trigger this checkpoint).
pub(crate) fn match_typed_decisions_workflow(ids: &BTreeSet<String>) -> Option<String> {
    const WORKFLOWS: &[(&str, &[&str])] = &[
        (
            "agent_trace_observability",
            &["action", "needs_review", "outcome", "risk", "urgency"],
        ),
        (
            "customer_service",
            &["action", "category", "churn_risk", "needs_human", "urgency"],
        ),
        (
            "invoice_processing",
            &[
                "discrepancy_severity",
                "disposition",
                "duplicate",
                "matches_order",
                "urgency",
            ],
        ),
        (
            "security_incidents",
            &[
                "credential_compromise",
                "disposition",
                "severity",
                "true_positive",
                "urgency",
            ],
        ),
    ];
    for (name, fields) in WORKFLOWS {
        let expected: BTreeSet<String> = fields.iter().map(|s| s.to_string()).collect();
        if *ids == expected {
            return Some((*name).to_string());
        }
    }
    None
}

/// Where a checkpoint's weights live: the bundle layout (`repo` for the
/// English root, `repo/subfolder` for the others) or the standalone
/// mirrors when the operator opted in.
pub(crate) fn repo_for(model: &str, standalone: bool) -> String {
    if standalone {
        return match model {
            "english" => BUNDLE_REPO.to_string(),
            "multilingual" => "convaiinnovations/laya-multilingual".to_string(),
            _ => "convaiinnovations/laya-typed-decisions".to_string(),
        };
    }
    match model {
        "english" => BUNDLE_REPO.to_string(),
        other => format!("{BUNDLE_REPO}/{other}"),
    }
}

/// The route verdict: which checkpoint reads this state, from where, and
/// WHY (the reason string is the audit-facing explanation).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct RouteDecision {
    pub model: String,
    pub repo: String,
    pub reason: String,
    pub detection: Option<Detection>,
    pub workflow: Option<String>,
}

/// The closed precedence chain — explicit model > explicit task > workflow
/// match (only when auto-detection is on) > explicit lang > the state's
/// script/language analysis > the default. Nothing here can widen the
/// vocabulary: every arm resolves to one of the three checkpoints. The
/// eight-parameter shape is the port contract's own signature (the
/// source's `route(state, questions, model?, task?, lang?, …)` keyword
/// set) — collapsed into a struct it would stop reading as the port.
#[allow(clippy::too_many_arguments)]
pub(crate) fn route(
    question_ids: &BTreeSet<String>,
    model: Option<&str>,
    task: Option<&str>,
    lang: Option<&str>,
    auto_task_detection: bool,
    default: &str,
    standalone: bool,
    detection: &Detection,
) -> Result<RouteDecision, String> {
    let repo_string = |model: &str| repo_for(model, standalone);

    if let Some(explicit) = model {
        let name = normalise_name(explicit)?;
        return Ok(RouteDecision {
            model: name.clone(),
            repo: repo_string(&name),
            reason: format!("explicit model={name:?}"),
            detection: None,
            workflow: None,
        });
    }
    if let Some(explicit_task) = task {
        let normalised = explicit_task.trim().to_lowercase().replace('-', "_");
        if normalised == "typed_decisions" {
            return Ok(RouteDecision {
                model: "typed-decisions".into(),
                repo: repo_string("typed-decisions"),
                reason: format!("explicit task={explicit_task:?}"),
                detection: None,
                workflow: None,
            });
        }
    }
    if auto_task_detection && let Some(workflow) = match_typed_decisions_workflow(question_ids) {
        return Ok(RouteDecision {
            model: "typed-decisions".into(),
            repo: repo_string("typed-decisions"),
            reason: format!("question ids match the {workflow} workflow"),
            detection: None,
            workflow: Some(workflow),
        });
    }
    if let Some(explicit_lang) = lang {
        let first = explicit_lang.split('-').next().unwrap_or("").to_lowercase();
        let name = if matches!(first.as_str(), "en" | "eng" | "english") {
            "english"
        } else {
            "multilingual"
        };
        return Ok(RouteDecision {
            model: name.into(),
            repo: repo_string(name),
            reason: format!("explicit lang={explicit_lang:?}"),
            detection: None,
            workflow: None,
        });
    }

    // The analysis arm: script is the primary signal.
    let default_name = normalise_name(default)?;
    if detection.script == "unknown" {
        return Ok(RouteDecision {
            model: default_name.clone(),
            repo: repo_string(&default_name),
            reason: format!("no letters detected in the state; using default ({default_name})"),
            detection: Some(detection.clone()),
            workflow: None,
        });
    }
    if detection.script != "latin" {
        let percent = detection.non_latin_fraction * 100.0;
        return Ok(RouteDecision {
            model: "multilingual".into(),
            repo: repo_string("multilingual"),
            reason: format!(
                "non-Latin script ({}, {:.1}% of letters); the English checkpoint cannot read it",
                detection.script, percent
            ),
            detection: Some(detection.clone()),
            workflow: None,
        });
    }
    if let Some(language) = &detection.language
        && language != "en"
    {
        return Ok(RouteDecision {
            model: "multilingual".into(),
            repo: repo_string("multilingual"),
            reason: format!("Latin script but language looks like {language}"),
            detection: Some(detection.clone()),
            workflow: None,
        });
    }
    Ok(RouteDecision {
        model: "english".into(),
        repo: repo_string("english"),
        reason: "English Latin text".into(),
        detection: Some(detection.clone()),
        workflow: None,
    })
}

/// The LRU bookkeeping over checkpoint names — pure state; the loader
/// (Phase 1) performs the actual (de)allocation this state describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LruState {
    pub max_loaded: usize,
    /// Most-recently-used first.
    pub order: Vec<String>,
    pub resident: Vec<String>,
}

impl LruState {
    pub fn new(max_loaded: usize) -> Self {
        Self {
            max_loaded: max_loaded.max(1),
            order: Vec::new(),
            resident: Vec::new(),
        }
    }

    pub fn touch(&mut self, name: &str) {
        self.order.retain(|n| n != name);
        if self.resident.contains(&name.to_string()) {
            self.order.insert(0, name.to_string());
        }
    }

    /// Evict least-recently-used residents until the cap holds. Eviction
    /// never touches a resident the operator attached (the cap was raised
    /// for it).
    pub fn evict_to_cap(&mut self) {
        while self.resident.len() > self.max_loaded {
            let Some(victim) = self.order.iter().rev().find(|n| self.resident.contains(n)) else {
                break;
            };
            let victim = victim.clone();
            self.resident.retain(|n| *n != victim);
            self.order.retain(|n| *n != victim);
        }
    }

    /// Raise the cap so every named checkpoint fits, then mark them
    /// resident (the preload posture: cold cost at boot, not per request).
    pub fn preload(&mut self, names: &[String]) {
        if names.len() > self.max_loaded {
            self.max_loaded = names.len();
        }
        for name in names {
            if !self.resident.contains(name) {
                self.resident.push(name.clone());
            }
            self.touch(name);
            self.evict_to_cap();
        }
    }

    pub fn unload(&mut self, name: Option<&str>) {
        match name {
            Some(one) => {
                self.resident.retain(|n| n != one);
                self.order.retain(|n| n != one);
            }
            None => {
                self.resident.clear();
                self.order.clear();
            }
        }
    }

    /// Register a prebuilt agent as resident; the cap rises to hold it.
    pub fn attach(&mut self, name: &str) {
        if !self.resident.contains(&name.to_string()) {
            self.resident.push(name.to_string());
        }
        self.touch(name);
        if self.resident.len() > self.max_loaded {
            self.max_loaded = self.resident.len();
        }
    }

    pub fn loaded(&self) -> &[String] {
        &self.resident
    }
}

/// The loading seam: the pure tests run against a stub; the Phase 1
/// inference module supplies the real ONNX loader.
pub(crate) trait CheckpointLoader {
    fn load(&self, state: &mut LruState, name: &str) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn detection_of(value: serde_json::Value) -> Detection {
        super::super::lang::analyse(&value)
    }

    fn ids(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // ── normalise_name / alias (9) ──────────────────────────────────────
    #[test]
    fn alias_en_laya_default_resolve_to_english() {
        for name in ["en", "laya", "default", "english"] {
            assert_eq!(normalise_name(name).unwrap(), "english", "{name}");
        }
    }
    #[test]
    fn alias_multi_ml_resolve_to_multilingual() {
        for name in ["multi", "ml", "laya-multilingual", "multilingual"] {
            assert_eq!(normalise_name(name).unwrap(), "multilingual", "{name}");
        }
    }
    #[test]
    fn alias_typed_family_resolves() {
        for name in ["typed", "typed_decisions", "decisions", "typed-decisions"] {
            assert_eq!(normalise_name(name).unwrap(), "typed-decisions", "{name}");
        }
    }
    #[test]
    fn alias_unknown_is_a_named_refusal() {
        let err = normalise_name("klingon").unwrap_err();
        assert!(err.contains("unknown checkpoint name"), "{err}");
    }
    #[test]
    fn alias_case_and_whitespace_normalise() {
        assert_eq!(normalise_name("  EN ").unwrap(), "english");
        assert_eq!(
            normalise_name("Typed-Decisions").unwrap(),
            "typed-decisions"
        );
    }
    #[test]
    fn alias_empty_refuses() {
        assert!(normalise_name("").is_err());
    }
    #[test]
    fn alias_checkpoints_table_is_the_three_names() {
        assert_eq!(CHECKPOINTS, &["english", "multilingual", "typed-decisions"]);
    }
    #[test]
    fn alias_every_table_name_normalises_to_itself() {
        for name in CHECKPOINTS {
            assert_eq!(normalise_name(name).unwrap(), *name);
        }
    }
    #[test]
    fn alias_bundle_repo_is_the_english_root() {
        assert_eq!(BUNDLE_REPO, "convaiinnovations/laya");
    }

    // ── match_typed_decisions_workflow (7) ──────────────────────────────
    #[test]
    fn workflow_agent_trace_exact_match() {
        assert_eq!(
            match_typed_decisions_workflow(&ids(&[
                "action",
                "needs_review",
                "outcome",
                "risk",
                "urgency"
            ])),
            Some("agent_trace_observability".into())
        );
    }
    #[test]
    fn workflow_customer_service_exact_match() {
        assert_eq!(
            match_typed_decisions_workflow(&ids(&[
                "action",
                "category",
                "churn_risk",
                "needs_human",
                "urgency"
            ])),
            Some("customer_service".into())
        );
    }
    #[test]
    fn workflow_invoice_processing_exact_match() {
        assert_eq!(
            match_typed_decisions_workflow(&ids(&[
                "discrepancy_severity",
                "disposition",
                "duplicate",
                "matches_order",
                "urgency"
            ])),
            Some("invoice_processing".into())
        );
    }
    #[test]
    fn workflow_security_incidents_exact_match() {
        assert_eq!(
            match_typed_decisions_workflow(&ids(&[
                "credential_compromise",
                "disposition",
                "severity",
                "true_positive",
                "urgency"
            ])),
            Some("security_incidents".into())
        );
    }
    #[test]
    fn workflow_partial_set_matches_nothing() {
        assert_eq!(
            match_typed_decisions_workflow(&ids(&["action", "urgency"])),
            None
        );
    }
    #[test]
    fn workflow_superset_matches_nothing() {
        assert_eq!(
            match_typed_decisions_workflow(&ids(&[
                "action",
                "category",
                "churn_risk",
                "needs_human",
                "urgency",
                "extra"
            ])),
            None
        );
    }
    #[test]
    fn workflow_empty_matches_nothing() {
        assert_eq!(match_typed_decisions_workflow(&BTreeSet::new()), None);
    }

    // ── route (14) ──────────────────────────────────────────────────────
    fn english_text() -> Detection {
        detection_of(json!(
            "the customer replaced the battery and it works fine now"
        ))
    }
    fn route_default(
        qids: &BTreeSet<String>,
        model: Option<&str>,
        task: Option<&str>,
        lang: Option<&str>,
        auto: bool,
        detection: &Detection,
    ) -> RouteDecision {
        route(qids, model, task, lang, auto, "english", false, detection).unwrap()
    }
    #[test]
    fn route_explicit_model_wins_over_everything() {
        let d = route_default(
            &BTreeSet::new(),
            Some("ml"),
            Some("typed_decisions"),
            Some("fr"),
            true,
            &english_text(),
        );
        assert_eq!(d.model, "multilingual");
        assert!(d.reason.contains("explicit model"));
    }
    #[test]
    fn route_explicit_model_normalises_and_binds_repo() {
        let d = route_default(
            &BTreeSet::new(),
            Some("typed"),
            None,
            None,
            false,
            &english_text(),
        );
        assert_eq!(d.model, "typed-decisions");
        assert_eq!(d.repo, "convaiinnovations/laya/typed-decisions");
    }
    #[test]
    fn route_explicit_model_unknown_refuses() {
        assert!(
            route(
                &BTreeSet::new(),
                Some("klingon"),
                None,
                None,
                false,
                "english",
                false,
                &english_text()
            )
            .is_err()
        );
    }
    #[test]
    fn route_explicit_task_typed_decisions() {
        let d = route_default(
            &BTreeSet::new(),
            None,
            Some("Typed-Decisions"),
            Some("fr"),
            false,
            &english_text(),
        );
        assert_eq!(d.model, "typed-decisions");
        assert!(d.reason.contains("explicit task"));
    }
    #[test]
    fn route_explicit_other_task_is_ignored() {
        let d = route_default(
            &BTreeSet::new(),
            None,
            Some("summarize"),
            None,
            false,
            &english_text(),
        );
        assert_eq!(d.model, "english");
        assert_eq!(d.reason, "English Latin text");
    }
    #[test]
    fn route_workflow_match_only_when_auto_detection_on() {
        let qids = ids(&["action", "category", "churn_risk", "needs_human", "urgency"]);
        let d = route_default(&qids, None, None, None, true, &english_text());
        assert_eq!(d.model, "typed-decisions");
        assert_eq!(d.workflow.as_deref(), Some("customer_service"));
        let off = route(
            &qids,
            None,
            None,
            None,
            false,
            "english",
            false,
            &english_text(),
        )
        .unwrap();
        assert_eq!(off.model, "english", "auto-detection off never matches");
    }
    #[test]
    fn route_explicit_lang_english_forms() {
        for lang in ["en", "eng", "english", "en-GB"] {
            let d = route_default(
                &BTreeSet::new(),
                None,
                None,
                Some(lang),
                false,
                &english_text(),
            );
            assert_eq!(d.model, "english", "{lang}");
            assert!(d.reason.contains("explicit lang"));
        }
    }
    #[test]
    fn route_explicit_lang_non_english_routes_multilingual() {
        let d = route_default(
            &BTreeSet::new(),
            None,
            None,
            Some("fr"),
            false,
            &english_text(),
        );
        assert_eq!(d.model, "multilingual");
    }
    #[test]
    fn route_unknown_script_defaults() {
        let d = route_default(
            &BTreeSet::new(),
            None,
            None,
            None,
            false,
            &detection_of(json!("123 456")),
        );
        assert_eq!(d.model, "english");
        assert!(d.reason.contains("no letters detected"));
    }
    #[test]
    fn route_non_latin_routes_multilingual_with_reason() {
        let d = route_default(
            &BTreeSet::new(),
            None,
            None,
            None,
            false,
            &detection_of(json!("клиент заменил батарею ноутбука")),
        );
        assert_eq!(d.model, "multilingual");
        assert!(d.reason.contains("non-Latin script"));
        assert!(d.reason.contains("cyrillic"));
    }
    #[test]
    fn route_latin_non_english_routes_multilingual() {
        let d = route_default(
            &BTreeSet::new(),
            None,
            None,
            None,
            false,
            &detection_of(json!(
                "le client a remplacé la batterie du portable et il marche bien"
            )),
        );
        assert_eq!(d.model, "multilingual");
        assert!(d.reason.contains("language looks like fr"));
    }
    #[test]
    fn route_english_latin_text_reads_english() {
        let d = route_default(&BTreeSet::new(), None, None, None, false, &english_text());
        assert_eq!(d.model, "english");
        assert_eq!(d.reason, "English Latin text");
    }
    #[test]
    fn route_custom_default_holds_for_unknown_scripts() {
        let d = route(
            &BTreeSet::new(),
            None,
            None,
            None,
            false,
            "multilingual",
            false,
            &detection_of(json!("123")),
        )
        .unwrap();
        assert_eq!(d.model, "multilingual");
    }
    #[test]
    fn route_precedence_model_beats_lang_beats_analysis() {
        let d = route_default(
            &BTreeSet::new(),
            None,
            None,
            Some("fr"),
            false,
            &detection_of(json!("клиент заменил батарею")),
        );
        assert_eq!(d.model, "multilingual", "lang and analysis agree here");
        let explicit = route_default(
            &BTreeSet::new(),
            Some("en"),
            None,
            Some("fr"),
            false,
            &detection_of(json!("клиент заменил батарею")),
        );
        assert_eq!(
            explicit.model, "english",
            "explicit model beats the lang arm"
        );
    }

    // ── decision shape (5) ──────────────────────────────────────────────
    #[test]
    fn decision_repo_matches_the_bundle_layout() {
        let d = route_default(
            &BTreeSet::new(),
            Some("multilingual"),
            None,
            None,
            false,
            &english_text(),
        );
        assert_eq!(d.repo, "convaiinnovations/laya/multilingual");
        let root = route_default(
            &BTreeSet::new(),
            Some("english"),
            None,
            None,
            false,
            &english_text(),
        );
        assert_eq!(root.repo, "convaiinnovations/laya");
    }
    #[test]
    fn decision_reason_is_always_present_and_binds_the_model() {
        for model in ["english", "multilingual", "typed-decisions"] {
            let d = route_default(
                &BTreeSet::new(),
                Some(model),
                None,
                None,
                false,
                &english_text(),
            );
            assert!(!d.reason.is_empty());
            assert!(d.reason.contains(model), "`{}` mentions {model}", d.reason);
        }
    }
    #[test]
    fn decision_detection_rides_only_the_analysis_arms() {
        let analysis = route_default(&BTreeSet::new(), None, None, None, false, &english_text());
        assert!(analysis.detection.is_some(), "the analysis arm carries it");
        let explicit = route_default(
            &BTreeSet::new(),
            Some("en"),
            None,
            None,
            false,
            &english_text(),
        );
        assert!(
            explicit.detection.is_none(),
            "explicit arms need no detection"
        );
    }
    #[test]
    fn decision_model_is_always_a_checkpoint_name() {
        for name in CHECKPOINTS {
            let d = route_default(
                &BTreeSet::new(),
                Some(name),
                None,
                None,
                false,
                &english_text(),
            );
            assert!(CHECKPOINTS.contains(&d.model.as_str()));
        }
    }
    #[test]
    fn decision_serializes_as_an_object() {
        let d = route_default(&BTreeSet::new(), None, None, None, false, &english_text());
        let value = serde_json::to_value(&d).unwrap();
        assert!(value.is_object());
        assert!(value["model"].is_string());
        assert!(value["reason"].is_string());
    }

    // ── bundle / standalone (8) ─────────────────────────────────────────
    #[test]
    fn bundle_english_is_the_repo_root() {
        assert_eq!(repo_for("english", false), "convaiinnovations/laya");
    }
    #[test]
    fn bundle_multilingual_is_a_subfolder() {
        assert_eq!(
            repo_for("multilingual", false),
            "convaiinnovations/laya/multilingual"
        );
    }
    #[test]
    fn bundle_typed_is_a_subfolder() {
        assert_eq!(
            repo_for("typed-decisions", false),
            "convaiinnovations/laya/typed-decisions"
        );
    }
    #[test]
    fn bundle_standalone_english_stays_the_root() {
        assert_eq!(repo_for("english", true), "convaiinnovations/laya");
    }
    #[test]
    fn bundle_standalone_mirrors_differ() {
        assert_eq!(
            repo_for("multilingual", true),
            "convaiinnovations/laya-multilingual"
        );
        assert_eq!(
            repo_for("typed-decisions", true),
            "convaiinnovations/laya-typed-decisions"
        );
    }
    #[test]
    fn bundle_vs_standalone_route_flags_flip_the_repo() {
        let qids = BTreeSet::new();
        let d = route(
            &qids,
            Some("multilingual"),
            None,
            None,
            false,
            "english",
            true,
            &english_text(),
        )
        .unwrap();
        assert_eq!(d.repo, "convaiinnovations/laya-multilingual");
    }
    #[test]
    fn bundle_override_models_map_through_normalise() {
        let d = route_default(
            &BTreeSet::new(),
            Some("laya-multilingual"),
            None,
            None,
            false,
            &english_text(),
        );
        assert_eq!(d.repo, "convaiinnovations/laya/multilingual");
    }
    #[test]
    fn bundle_standalone_map_is_total_over_checkpoints() {
        for name in CHECKPOINTS {
            let repo = repo_for(name, true);
            assert!(!repo.is_empty());
            assert!(repo.starts_with("convaiinnovations/"));
        }
    }

    // ── LRU (6) ─────────────────────────────────────────────────────────
    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn lru_cap_one_evicts_on_new_resident() {
        // The load path (what the Phase 1 loader wraps): resident + touch +
        // evict-to-cap. attach is the OTHER entry — it raises the cap.
        let mut lru = LruState::new(1);
        lru.resident.push("english".into());
        lru.touch("english");
        lru.resident.push("multilingual".into());
        lru.touch("multilingual");
        lru.evict_to_cap();
        assert_eq!(lru.loaded(), &names(&["multilingual"]));
    }
    #[test]
    fn lru_cap_two_holds_two() {
        let mut lru = LruState::new(2);
        lru.attach("english");
        lru.attach("multilingual");
        assert_eq!(lru.loaded().len(), 2);
        assert_eq!(lru.max_loaded, 2);
    }
    #[test]
    fn lru_touch_moves_to_front_and_protects_from_eviction() {
        let mut lru = LruState::new(2);
        for n in ["english", "multilingual"] {
            lru.resident.push(n.into());
            lru.touch(n);
            lru.evict_to_cap();
        }
        // The re-touch makes english most-recently-used again, so the next
        // resident evicts multilingual (the LRU), not english.
        lru.touch("english");
        lru.resident.push("typed-decisions".into());
        lru.touch("typed-decisions");
        lru.evict_to_cap();
        assert_eq!(lru.loaded(), &names(&["english", "typed-decisions"]));
    }
    #[test]
    fn lru_unload_one_and_all() {
        let mut lru = LruState::new(2);
        lru.attach("english");
        lru.attach("multilingual");
        lru.unload(Some("english"));
        assert_eq!(lru.loaded(), &names(&["multilingual"]));
        lru.unload(None);
        assert!(lru.loaded().is_empty());
    }
    #[test]
    fn lru_never_falls_below_one() {
        let lru = LruState::new(0);
        assert_eq!(lru.max_loaded, 1, "max_loaded clamps to >= 1");
    }
    #[test]
    fn lru_touch_of_absent_name_is_a_no_op() {
        let mut lru = LruState::new(2);
        lru.attach("english");
        lru.touch("multilingual");
        assert_eq!(lru.loaded(), &names(&["english"]));
        assert_eq!(lru.order.len(), 1);
    }

    // ── preload (4) ─────────────────────────────────────────────────────
    #[test]
    fn preload_all_three_raises_the_cap() {
        let mut lru = LruState::new(1);
        lru.preload(&names(&["english", "multilingual", "typed-decisions"]));
        assert_eq!(lru.loaded().len(), 3);
        assert_eq!(lru.max_loaded, 3);
    }
    #[test]
    fn preload_subset_holds_under_the_cap() {
        let mut lru = LruState::new(2);
        lru.preload(&names(&["english", "multilingual"]));
        assert_eq!(lru.loaded(), &names(&["english", "multilingual"]));
        assert_eq!(lru.max_loaded, 2);
    }
    #[test]
    fn preload_touch_keeps_priority_and_never_evicts_within_cap() {
        let mut lru = LruState::new(3);
        lru.preload(&names(&["typed-decisions", "english", "multilingual"]));
        lru.touch("typed-decisions");
        assert_eq!(lru.loaded().len(), 3, "nothing evicts within the cap");
        assert_eq!(
            lru.order.first().map(String::as_str),
            Some("typed-decisions")
        );
    }
    #[test]
    fn preload_is_idempotent() {
        let mut lru = LruState::new(2);
        let set = names(&["english", "multilingual"]);
        lru.preload(&set);
        lru.preload(&set);
        assert_eq!(lru.loaded(), &set);
        assert_eq!(lru.max_loaded, 2);
    }

    // ── attach (5) ──────────────────────────────────────────────────────
    #[test]
    fn attach_registers_a_prebuilt_agent() {
        let mut lru = LruState::new(1);
        lru.attach("english");
        assert_eq!(lru.loaded(), &names(&["english"]));
    }
    #[test]
    fn attach_bumps_the_cap() {
        let mut lru = LruState::new(1);
        lru.attach("english");
        lru.attach("multilingual");
        assert_eq!(lru.max_loaded, 2, "the cap rises to hold what arrived");
        assert_eq!(lru.loaded().len(), 2);
    }
    #[test]
    fn attach_then_touch_orders_front() {
        let mut lru = LruState::new(3);
        lru.attach("english");
        lru.attach("multilingual");
        lru.touch("english");
        assert_eq!(lru.order.first().map(String::as_str), Some("english"));
    }
    #[test]
    fn attach_then_unload_frees_exactly_one() {
        let mut lru = LruState::new(2);
        lru.attach("english");
        lru.attach("multilingual");
        lru.unload(Some("english"));
        assert_eq!(lru.loaded(), &names(&["multilingual"]));
    }
    #[test]
    fn attach_twice_is_a_no_op_second_time() {
        let mut lru = LruState::new(1);
        lru.attach("english");
        lru.attach("english");
        assert_eq!(lru.loaded().len(), 1);
        assert_eq!(lru.max_loaded, 1);
    }

    // ── the loader seam ─────────────────────────────────────────────────
    struct StubLoader {
        allowed: Vec<String>,
    }
    impl CheckpointLoader for StubLoader {
        fn load(&self, state: &mut LruState, name: &str) -> Result<(), String> {
            if !self.allowed.contains(&name.to_string()) {
                return Err(format!("checkpoint not available: {name}"));
            }
            // The load path: becomes resident, most-recently-used, and the
            // cap evicts the LRU — never the attach path (that one bumps).
            if !state.resident.contains(&name.to_string()) {
                state.resident.push(name.to_string());
            }
            state.touch(name);
            state.evict_to_cap();
            Ok(())
        }
    }
    #[test]
    fn loader_stub_drives_the_lru_without_ort() {
        let loader = StubLoader {
            allowed: vec!["english".into(), "multilingual".into()],
        };
        let mut lru = LruState::new(1);
        loader.load(&mut lru, "english").unwrap();
        loader.load(&mut lru, "english").unwrap();
        loader.load(&mut lru, "multilingual").unwrap();
        assert_eq!(lru.loaded(), &names(&["multilingual"]));
        assert!(loader.load(&mut lru, "typed-decisions").is_err());
    }
}
