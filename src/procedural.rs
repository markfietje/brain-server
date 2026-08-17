//! deterministic procedural-memory primitives.
//!
//! The brain of an AI consultant: stores and reasons over assessment playbooks,
//! decision trees, vendor knowledge, and ordered implementation steps. Every
//! operation here is **deterministic** — no LLM, no cloud, no tokens. This is
//! the wedge: Mem0 charges tokens to auto-categorize; we do it with a keyword
//! router. Graphiti's `NextEpisodeEdge` ships as a typed edge here.
//!
//! Research basis (Context7, 2026-08-02):
//!   - Mem0: `procedural_memory` type + 15 auto-categories (LLM-driven there).
//!   - Graphiti: `NextEpisodeEdge` for ordered steps.
//!   - Letta: HITL approval gates on risky steps (execution is the agent's job;
//!     the brain stores + retrieves + evaluates, never executes).

// ─────────────────────────────────────────────────────────────────────────
// memory_kind — the classification Mem0 sells as a premium cloud feature,
// done here deterministically via a bounded keyword router.
// ─────────────────────────────────────────────────────────────────────────

/// The four memory classes a consultant's brain carries. Stored on the
/// v1.4-reserved `knowledge.node_kind` column (repurposed in v1.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    /// A declarative statement ("Acme Corp uses QuickBooks"). Default.
    Fact,
    /// An ordered runbook/playbook root ("Small-business AI readiness assessment").
    Procedure,
    /// One ordered step within a procedure ("1. Inventory current software stack").
    Step,
    /// A conditional branch — the consultant's core reasoning primitive
    /// ("If HIPAA-relevant → recommend BAA-reviewed tools").
    Decision,
    /// a dated event record where `observed_at` is
    /// first-class (an episodic memory). The natural TTL candidate (M2).
    Episodic,
}

impl MemoryKind {
    /// Stable string for SQL. Matches the `node_kind` column values.
    pub const fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Fact => "fact",
            MemoryKind::Procedure => "procedure",
            MemoryKind::Step => "step",
            MemoryKind::Decision => "decision",
            MemoryKind::Episodic => "episodic",
        }
    }
    /// Parse from the stored string. Unknown values fall back to `Fact`
    /// (forward-compat: a future kind we don't know about still answers
    /// `/recall` as a declarative chunk).
    pub fn from_str(s: &str) -> Self {
        match s.trim() {
            "procedure" => MemoryKind::Procedure,
            "step" => MemoryKind::Step,
            "decision" => MemoryKind::Decision,
            "episodic" => MemoryKind::Episodic,
            _ => MemoryKind::Fact,
        }
    }
/// the strict write-boundary validator. A
    /// kind string is valid iff it round-trips through [`Self::from_str`] —
    /// `from_str` falls back to `Fact` on any unknown/mixed-case input, which
    /// must never be *silently accepted* at the write boundary (both the
    /// proposal path and `/ingest` hard-reject with 400 instead).
    pub fn is_strict_valid(s: &str) -> bool {
        let s = s.trim();
        !s.is_empty() && Self::from_str(s).as_str() == s
    }
}

/// All valid category labels (used by `/classify` to advertise the taxonomy).
pub fn categories() -> &'static [&'static str] {
    CATEGORIES
}

// ─────────────────────────────────────────────────────────────────────────
// classify — deterministic categorization (Mem0's premium feature, free).
// ─────────────────────────────────────────────────────────────────────────

/// A bounded set of consultant-relevant categories. Mem0's 15 default buckets
/// (personal_details, sports, food, ...) are consumer-oriented; this set is
/// tuned to the small-business AI-transformation domain. The keyword router
/// is deliberately conservative — it returns `general` (no strong signal)
/// rather than guessing, so the classification is always defensible.
pub const CATEGORIES: &[&str] = &[
    "technology",
    "business_process",
    "compliance",
    "finance",
    "vendor",
    "assessment",
    "infrastructure",
    "general",
];

/// Categorize a text deterministically. Returns the **highest-scoring**
/// category (or `general` when no keyword clears the threshold). This is the
/// lazy substitute for an LLM classifier: a keyword router with a tiny,
/// hand-curated lexicon. It will never be as smart as a fine-tuned model, but
/// it is (a) free, (b) local, (c) deterministic, (d) auditable — every match
/// is traceable to a specific keyword, which matters for a consultant whose
/// recommendations must be defensible.
///
/// `ponytail:` ceiling: keyword matching is O(text × lexicon). Fine for chunk-
/// sized inputs (≤ MAX_CONTENT); a corpus-wide re-classification would want an
/// inverted index. Upgrade path: a model2vec custom-vocab classifier (v1.11).
pub fn classify(text: &str) -> CategoryResult {
    let lower = text.to_lowercase();
    let mut scores: [(usize, &str); 7] = [0; 7].map(|_| (0, ""));
    //Lexicon is small + hand-curated; one keyword = one vote.
    for (i, cat) in [
        "technology",
        "business_process",
        "compliance",
        "finance",
        "vendor",
        "assessment",
        "infrastructure",
    ]
    .iter()
    .enumerate()
    {
        let kw = LEXICON[i];
        let mut hits = 0usize;
        for k in kw {
            if lower.contains(k) {
                hits += 1;
            }
        }
        scores[i] = (hits, cat);
    }
    scores.sort_by_key(|s| std::cmp::Reverse(s.0));
    let (best_hits, best_cat) = scores[0];
    if best_hits == 0 {
        return CategoryResult {
            category: "general",
            confidence: 0.0,
            matched_keywords: Vec::new(),
        };
    }
    // Confidence = best category's hits / total non-zero hits. A text that's
    // 100% finance keywords scores 1.0; a 50/50 split scores 0.5.
    let total: usize = scores.iter().map(|(h, _)| *h).sum();
    let confidence = if total == 0 {
        0.0
    } else {
        best_hits as f32 / total as f32
    };
    // Resolve the lexicon index from the category, not from `scores`: the
    // sort above reorders the array, so its slot no longer equals the LEXICON
    // index (that bug surfaced as `classify_detects_compliance` failing to
    // report its `hipaa` match). CATEGORIES[0..7] mirrors LEXICON order.
    let lex_idx = CATEGORIES.iter().position(|c| *c == best_cat).unwrap_or(0);
    let matched_keywords = LEXICON[lex_idx]
        .iter()
        .filter(|k| lower.contains(**k))
        .map(|s| s.to_string())
        .collect();
    CategoryResult {
        category: best_cat,
        confidence,
        matched_keywords,
    }
}

/// Result of a classification. `matched_keywords` makes the decision auditable
/// — a consultant can see *why* the brain called this "compliance".
#[derive(Debug, Clone, serde::Serialize)]
pub struct CategoryResult {
    pub category: &'static str,
    /// In `[0.0, 1.0]`. `0.0` for `general` (no signal).
    pub confidence: f32,
    /// The keywords that fired for the winning category. Empty for `general`.
    pub matched_keywords: Vec<String>,
}

/// The lexicon. Hand-curated, domain-tuned. Order matches `CATEGORIES` (minus
/// `general`, which is the fallback). Kept tiny on purpose — a bigger lexicon
/// would need versioning + tests; this is the smallest set that covers the
/// consultant's vocabulary.
const LEXICON: &[&[&str]] = &[
    // technology
    &[
        "ai",
        "ml",
        "llm",
        "api",
        "cloud",
        "saas",
        "automation",
        "integration",
        "software",
        "model",
    ],
    // business_process
    &[
        "workflow",
        "process",
        "operations",
        "efficiency",
        "manual",
        "repetitive",
        "onboarding",
        "approval",
        "handoff",
    ],
    // compliance
    &[
        "hipaa",
        "gdpr",
        "pci",
        "soc2",
        "pii",
        "privacy",
        "audit",
        "regulation",
        "retention",
        "consent",
    ],
    // finance
    &[
        "budget",
        "roi",
        "cost",
        "revenue",
        "invoice",
        "quickbooks",
        "accounting",
        "margin",
        "spend",
        "pricing",
    ],
    // vendor
    &[
        "openai",
        "anthropic",
        "microsoft",
        "google",
        "aws",
        "azure",
        "subscription",
        "vendor",
        "tool",
        "platform",
    ],
    // assessment
    &[
        "readiness",
        "maturity",
        "assess",
        "evaluate",
        "score",
        "rubric",
        "gap",
        "opportunity",
        "recommend",
        "fit",
    ],
    // infrastructure
    &[
        "server",
        "network",
        "storage",
        "backup",
        "vmware",
        "proxmox",
        "linux",
        "database",
        "kubernetes",
        "deploy",
    ],
];

// ─────────────────────────────────────────────────────────────────────────
// decision — deterministic rule evaluation (the consultant's reasoning core).
// ─────────────────────────────────────────────────────────────────────────

/// A decision rule. Stored as JSON in the `content` of a `decision`-kind chunk.
/// Bounded DSL: a list of conditions, each mapping to a branch label. The first
/// matching condition wins (deterministic ordering). If none match, the
/// `default_branch` is returned. This is NOT Prolog — it's the smallest rule
/// engine that makes a consultant's decision-tree expertise queryable.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecisionRule {
    /// Human-readable description ("Which AI-readiness tier does this client belong to?").
    pub description: String,
    /// Ordered conditions; first match wins. Each is `<variable> <op> <value>`.
    pub branches: Vec<DecisionBranch>,
    /// The branch taken when no condition matches. Always present so the
    /// result is total (never "no answer").
    pub default_branch: String,
}

/// One branch of a decision rule. `condition` is a simple triple
/// (`variable op value`, e.g. `employee_count >= 50`); `result` is the label
/// returned when this branch fires.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecisionBranch {
    pub condition: String,
    pub result: String,
    /// Optional citation — a chunk id whose content justifies this branch.
    /// The consultant's "why" pointer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<i64>,
}

/// Evaluate a decision rule against a set of input variables. Returns the
/// matched branch's result (or the default) + the citation chain. Pure so the
/// rule engine can be unit-tested without a database.
///
/// Variables are a flat `HashMap<String, f64>` — numeric comparisons only.
/// String-equality branches would need a richer type; deferred (the consultant's
/// rubrics are almost always numeric thresholds: employee count, revenue, score).
pub fn evaluate_decision(
    rule: &DecisionRule,
    vars: &std::collections::HashMap<String, f64>,
) -> DecisionOutcome {
    for branch in &rule.branches {
        if let Some((var, op, val)) = parse_condition(&branch.condition) {
            if let Some(actual) = vars.get(var) {
                if matches_op(*actual, op, val) {
                    return DecisionOutcome {
                        result: branch.result.clone(),
                        matched_condition: Some(branch.condition.clone()),
                        citation: branch.citation,
                        used_default: false,
                    };
                }
            }
        }
    }
    DecisionOutcome {
        result: rule.default_branch.clone(),
        matched_condition: None,
        citation: None,
        used_default: true,
    }
}

/// The outcome of evaluating a decision rule. Carries the citation chain so
/// the consultant can defend the recommendation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DecisionOutcome {
    pub result: String,
    /// The condition that fired (None when the default was taken).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_condition: Option<String>,
    /// Chunk id justifying this branch (the "why").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<i64>,
    pub used_default: bool,
}

/// Parse a condition triple: `"employee_count >= 50"` → `("employee_count", ">=", 50.0)`.
/// Returns None on any parse failure (the rule is then skipped — a malformed
/// condition never fires, which is safer than guessing).
pub fn parse_condition(cond: &str) -> Option<(&str, &str, f64)> {
    let cond = cond.trim();
    // Find the operator. Supported: >=, <=, !=, ==, >, <. Order matters: the
    // two-char ops must be checked before the one-char ones.
    for op in [">=", "<=", "!=", "==", ">", "<"] {
        if let Some((var, val)) = cond.split_once(op) {
            let var = var.trim();
            let val = val.trim().parse::<f64>().ok()?;
            if var.is_empty() {
                return None;
            }
            return Some((var, op, val));
        }
    }
    None
}

/// Apply a comparison operator. `==`/`!=` use exact f64 equality — fine for
/// the consultant's rubrics (integer thresholds); floating-point equality
/// would need an epsilon, but no real rule uses fractional comparisons.
pub fn matches_op(actual: f64, op: &str, expected: f64) -> bool {
    match op {
        ">=" => actual >= expected,
        "<=" => actual <= expected,
        ">" => actual > expected,
        "<" => actual < expected,
        "==" => actual == expected,
        "!=" => actual != expected,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ── memory_kind ───────────────────────────────────────────────────────

    #[test]
    fn memory_kind_round_trips() {
        for k in [
            MemoryKind::Fact,
            MemoryKind::Procedure,
            MemoryKind::Step,
            MemoryKind::Decision,
        ] {
            assert_eq!(MemoryKind::from_str(k.as_str()), k);
        }
    }

    #[test]
    fn memory_kind_unknown_falls_back_to_fact() {
        assert_eq!(MemoryKind::from_str("session"), MemoryKind::Fact); // legacy v1.4 value
        assert_eq!(MemoryKind::from_str("nonsense"), MemoryKind::Fact);
        assert_eq!(MemoryKind::from_str(""), MemoryKind::Fact);
    }

    // ── classify ─────────────────────────────────────────────────────────

    #[test]
    fn classify_returns_general_when_no_keyword_matches() {
        let r = classify("the cat sat on the mat");
        assert_eq!(r.category, "general");
        assert_eq!(r.confidence, 0.0);
        assert!(r.matched_keywords.is_empty());
    }

    #[test]
    fn classify_detects_compliance() {
        let r = classify("This client handles patient records; HIPAA and PII apply.");
        assert_eq!(r.category, "compliance");
        assert!(r.confidence > 0.0);
        assert!(r.matched_keywords.iter().any(|k| k == "hipaa"));
        assert!(r.matched_keywords.iter().any(|k| k == "pii"));
    }

    #[test]
    fn classify_detects_technology() {
        let r = classify("We need an LLM integration via their SaaS API.");
        assert_eq!(r.category, "technology");
        assert!(r.matched_keywords.iter().any(|k| k == "llm"));
    }

    #[test]
    fn classify_detects_finance() {
        let r = classify("ROI is 3x; budget is $50k; they use QuickBooks.");
        assert_eq!(r.category, "finance");
        assert!(r.confidence > 0.0);
    }

    #[test]
    fn classify_is_case_insensitive() {
        assert_eq!(classify("HIPAA HIPAA hipaa").category, "compliance");
        assert_eq!(classify("AWS azure AZURE").category, "vendor");
    }

    #[test]
    fn classify_confidence_is_winning_fraction() {
        // 2 technology keywords + 1 finance keyword → technology at 2/3 ≈ 0.667.
        let r = classify("AI automation budget");
        assert_eq!(r.category, "technology");
        assert!((r.confidence - (2.0 / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn classify_vendor_lexicon_matches_real_vendors() {
        assert_eq!(classify("OpenAI vs Anthropic vs Google").category, "vendor");
        assert_eq!(classify("They're on Microsoft Azure").category, "vendor");
    }

    // ── decision evaluation ──────────────────────────────────────────────

    fn tier_rule() -> DecisionRule {
        DecisionRule {
            description: "AI-readiness tier".into(),
            branches: vec![
                DecisionBranch {
                    condition: "employee_count >= 50".into(),
                    result: "enterprise".into(),
                    citation: Some(42),
                },
                DecisionBranch {
                    condition: "employee_count >= 10".into(),
                    result: "mid-market".into(),
                    citation: None,
                },
            ],
            default_branch: "small-business".into(),
        }
    }

    #[test]
    fn decision_first_matching_branch_wins() {
        let rule = tier_rule();
        let mut vars = HashMap::new();
        vars.insert("employee_count".into(), 75.0);
        let out = evaluate_decision(&rule, &vars);
        assert_eq!(out.result, "enterprise");
        assert!(!out.used_default);
        assert_eq!(out.citation, Some(42));
        assert_eq!(
            out.matched_condition.as_deref(),
            Some("employee_count >= 50")
        );
    }

    #[test]
    fn decision_second_branch_when_first_misses() {
        let rule = tier_rule();
        let mut vars = HashMap::new();
        vars.insert("employee_count".into(), 25.0);
        let out = evaluate_decision(&rule, &vars);
        assert_eq!(out.result, "mid-market");
        assert!(!out.used_default);
    }

    #[test]
    fn decision_default_when_no_branch_matches() {
        let rule = tier_rule();
        let mut vars = HashMap::new();
        vars.insert("employee_count".into(), 3.0);
        let out = evaluate_decision(&rule, &vars);
        assert_eq!(out.result, "small-business");
        assert!(out.used_default);
        assert!(out.citation.is_none());
    }

    #[test]
    fn decision_missing_variable_falls_through_to_default() {
        let rule = tier_rule();
        let vars = HashMap::new(); // no employee_count
        let out = evaluate_decision(&rule, &vars);
        assert!(out.used_default);
        assert_eq!(out.result, "small-business");
    }

    // ── condition parsing ────────────────────────────────────────────────

    #[test]
    fn parse_condition_handles_all_operators() {
        assert_eq!(parse_condition("x >= 5"), Some(("x", ">=", 5.0)));
        assert_eq!(parse_condition("x <= 5"), Some(("x", "<=", 5.0)));
        assert_eq!(parse_condition("x > 5"), Some(("x", ">", 5.0)));
        assert_eq!(parse_condition("x < 5"), Some(("x", "<", 5.0)));
        assert_eq!(parse_condition("x == 5"), Some(("x", "==", 5.0)));
        assert_eq!(parse_condition("x != 5"), Some(("x", "!=", 5.0)));
    }

    #[test]
    fn parse_condition_rejects_garbage() {
        assert!(parse_condition("no operator here").is_none());
        assert!(parse_condition("x = 5").is_none()); // single = is not an op
        assert!(parse_condition(">= 5").is_none()); // no variable
        assert!(parse_condition("x >= abc").is_none()); // non-numeric value
    }

    #[test]
    fn parse_condition_two_char_ops_take_precedence() {
        // "x >= 5" must parse as >= not >. If we checked ">" first we'd get var "x " op ">" val "= 5" → fail.
        let p = parse_condition("score >= 0.8");
        assert_eq!(p, Some(("score", ">=", 0.8)));
    }

    #[test]
    fn matches_op_all_six_comparisons() {
        assert!(matches_op(5.0, ">=", 5.0));
        assert!(matches_op(6.0, ">", 5.0));
        assert!(!matches_op(5.0, ">", 5.0));
        assert!(matches_op(5.0, "==", 5.0));
        assert!(matches_op(5.0, "!=", 6.0));
        assert!(matches_op(4.0, "<", 5.0));
        assert!(matches_op(5.0, "<=", 5.0));
        assert!(!matches_op(7.0, "<=", 5.0));
    }
}
