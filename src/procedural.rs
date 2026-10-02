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
    /// first-class (an episodic memory). The natural TTL candidate.
    Episodic,
    /// Governed warranty/contract state (the Frontdesk entitlement
    /// registry): product/serial traceability + coverage windows. Created
    /// ONLY via proposals, like every knowledge write.
    Entitlement,
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
            MemoryKind::Entitlement => "entitlement",
        }
    }
    /// Parse from the stored string. Unknown values fall back to `Fact`
    /// (forward-compat: a future kind we don't know about still answers
    /// `/recall` as a declarative chunk).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.trim() {
            "procedure" => MemoryKind::Procedure,
            "step" => MemoryKind::Step,
            "decision" => MemoryKind::Decision,
            "episodic" => MemoryKind::Episodic,
            "entitlement" => MemoryKind::Entitlement,
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
///
/// **`dell_support` is a POOL, not a peer.** The eight above classify an
/// ENGAGEMENT — its commercial posture, its compliance surface, whether an
/// assessment is happening. `dell_support` classifies the SUBJECT MATTER: the
/// case is about Dell/EMC infrastructure. Those are different axes, which is
/// exactly why the earlier refusal held: the 33 fault families (`vsan`,
/// `xe-gpu`, `san-fabric`, ...) had no home in a set containing `finance`.
///
/// It is added here rather than beside the eight because the classifier emits
/// exactly one category, so a pool must occupy a slot to be emittable. It sits
/// LAST, immediately before `general`, so the fallback stays the fallback and
/// no existing index moves — `CATEGORIES[0..7]` is mirrored by `LEXICON` order
/// and by `RoutingClass`'s discriminants, and appending is the only edit that
/// leaves all three intact.
pub const CATEGORIES: &[&str] = &[
    "technology",
    "business_process",
    "compliance",
    "finance",
    "vendor",
    "assessment",
    "infrastructure",
    "dell_support",
    "general",
];

/// Whole-word keyword membership. `keyword` fires iff it equals one of `text`'s
/// tokens — `text` is split on non-alphanumeric characters and compared
/// case-insensitively.
///
/// Substring matching (`lower.contains(k)`) let a single spurious hit manufacture a
/// perfect score: `"ai"` fired inside `"email"`, `"said"`, `"detail"` and
/// `"maintain"`, and — worst — inside `"openai"`, a *different* lexicon entry,
/// so a vendor name voted for the technology category. The share of fired
/// keywords then reported that fabricated hit as an uncontested `1.0`.
///
/// Splitting on non-alphanumeric characters keeps both directions honest:
/// `ai-assistant` and `AI-driven` still fire `ai`, while `openai` does not.
fn keyword_fires(tokens: &[String], keyword: &str) -> bool {
    tokens.iter().any(|t| t == keyword)
}

/// Tokenize for keyword matching: maximal runs of alphanumeric characters,
/// lowercased. Shared by the scoring loop and the matched-keyword report so the
/// two can never disagree about what fired.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Categorize a text deterministically. Returns the **highest-scoring**
/// category (or `general` when no keyword clears the threshold). This is the
/// lazy substitute for an LLM classifier: a keyword router with a tiny,
/// hand-curated lexicon. It will never be as smart as a fine-tuned model, but
/// it is (a) free, (b) local, (c) deterministic, (d) auditable — every match
/// is traceable to a specific keyword, which matters for a consultant whose
/// recommendations must be defensible.
///
/// **Ceiling on the returned `confidence`:** it is the winning category's *share*
/// of the keywords that fired, not a calibrated probability. One keyword firing
/// uncontested scores `1.0` exactly as ten do — [`CategoryResult::evidence_count`]
/// is what separates those two cases, and a consumer must read both.
///
/// `ponytail:` ceiling: keyword matching is O(text × lexicon). Fine for chunk-
/// sized inputs (≤ MAX_CONTENT); a corpus-wide re-classification would want an
/// inverted index. Upgrade path: a model2vec custom-vocab classifier (v1.11).
pub fn classify(text: &str) -> CategoryResult {
    let tokens = tokenize(text);
    // One entry per scored category, derived from CATEGORIES (see
    // `scored_categories`) so the array can never drift from the vocabulary.
    let cats: Vec<&'static str> = scored_categories().collect();
    let mut scores: Vec<(usize, &str)> = Vec::with_capacity(cats.len());
    //Lexicon is small + hand-curated; one keyword = one vote.
    for (i, cat) in cats.iter().enumerate() {
        let kw = LEXICON[i];
        let mut hits = 0usize;
        for k in kw {
            if keyword_fires(&tokens, k) {
                hits += 1;
            }
        }
        scores.push((hits, cat));
    }
    scores.sort_by_key(|s| std::cmp::Reverse(s.0));
    let (best_hits, best_cat) = scores[0];
    if best_hits == 0 {
        return CategoryResult {
            category: "general",
            confidence: 0.0,
            evidence_count: 0,
            matched_keywords: Vec::new(),
        };
    }
    // Confidence = best category's hits / total non-zero hits. A text that's
    // 100% finance keywords scores 1.0; a 50/50 split scores 0.5. This is an
    // UNCONTESTED SHARE, not a confidence: `evidence_count` is the companion
    // signal, because both 1-of-1 and 10-of-10 read as 1.0 here.
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
    let matched_keywords: Vec<String> = LEXICON[lex_idx]
        .iter()
        .filter(|k| keyword_fires(&tokens, k))
        .map(|s| s.to_string())
        .collect();
    CategoryResult {
        category: best_cat,
        confidence,
        // The count a threshold actually needs, and the honest answer to "how
        // much evidence stands behind this verdict". Always equal to
        // `matched_keywords.len()` by construction — the count is derived from
        // the same scan, never counted independently.
        evidence_count: best_hits,
        matched_keywords,
    }
}

/// Result of a classification. `matched_keywords` makes the decision auditable
/// — a consultant can see *why* the brain called this "compliance".
#[derive(Debug, Clone, serde::Serialize)]
pub struct CategoryResult {
    pub category: &'static str,
    /// In `[0.0, 1.0]`. `0.0` for `general` (no signal).
    ///
    /// **This is the winning category's share of the fired keywords, not a
    /// calibrated probability.** One keyword firing uncontested and ten keywords
    /// all agreeing both read `1.0` here; [`Self::evidence_count`] is what tells
    /// them apart, so a consumer must read both fields together.
    pub confidence: f32,
    /// How many keywords fired for the winning category — the evidence standing
    /// behind the verdict, and the quantity a threshold actually keys on. `0` for
    /// `general`, and always equal to `matched_keywords.len()`.
    pub evidence_count: usize,
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
    // dell_support — the SUBJECT-MATTER pool, not an engagement category.
    // These are the product-line and symptom words that distinguish a Dell/EMC
    // support case from a generic infrastructure one. `infrastructure` above
    // holds `server`/`network`/`storage` on purpose: it is the ENGAGEMENT-side
    // category, and a Dell case may legitimately score on both. The winner is
    // decided by share of fired keywords, so a case that names a Dell product
    // line outranks a case that merely says "server".
    //
    // Coverage is the CORPUS vocabulary, taken from the 33 fault-family tokens
    // in the routing matrix: the hardware lines (idrac, perc, poweredge, idrac9,
    // powerstore, powerscale, powermax, powervault, unity, vnx, dell, xeon, xe),
    // the stacks (vsan, vxrail, vcf, nsx, esxi, vcenter, vcsa, powerflex,
    // powermax, scaleio), and the storage/peripheral nouns (san, nas, fibre/
    // fiber, tape, rma, dimm, raid, nvme).
    &[
        "dell",
        "emc",
        "poweredge",
        "idrac",
        "perc",
        "raid",
        "nvme",
        "dimm",
        "xeon",
        "powerstore",
        "powerscale",
        "powermax",
        "powervault",
        "unity",
        "vnx",
        "clariion",
        "powerflex",
        "scaleio",
        "vsan",
        "vxrail",
        "vcf",
        "nsx",
        "esxi",
        "esx",
        "vcenter",
        "vcsa",
        "san",
        "nas",
        "fibre",
        "fiber",
        "tape",
        "rma",
    ],
];

/// The scoring categories: `CATEGORIES` minus the `general` fallback.
///
/// DERIVED from `CATEGORIES` rather than restated. The classifier used to score
/// a hard-coded `[7]` array against a separately-typed list of the same seven
/// names — a second copy of the vocabulary, in the one function whose whole job
/// is to read that vocabulary, and it had already drifted once (the array was
/// widened to 7 while `LEXICON` still had 7 entries, leaving `general`
/// unreachable as a scored category). Deriving it means a category added to
/// `CATEGORIES` with a matching `LEXICON` entry is scored automatically.
fn scored_categories() -> impl Iterator<Item = &'static str> {
    CATEGORIES.iter().copied().filter(|c| *c != "general")
}

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
        if let Some((var, op, val)) = parse_condition(&branch.condition)
            && let Some(actual) = vars.get(var)
            && matches_op(*actual, op, val)
        {
            return DecisionOutcome {
                result: branch.result.clone(),
                matched_condition: Some(branch.condition.clone()),
                citation: branch.citation,
                used_default: false,
            };
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
            MemoryKind::Episodic,
            MemoryKind::Entitlement,
        ] {
            assert_eq!(MemoryKind::from_str(k.as_str()), k);
            assert!(MemoryKind::is_strict_valid(k.as_str()));
        }
        assert!(!MemoryKind::is_strict_valid("Entitlement"));
        assert!(!MemoryKind::is_strict_valid("entitlements"));
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

    // ── dell_support: the subject-matter pool ─────────────────────────────

    /// The pool FIRES. A category nothing emits is a category that routes
    /// nowhere, which is exactly what the routing-axis drift pin exists to
    /// prevent — so the new variant needs a positive proof, not just the pin.
    #[test]
    fn classify_detects_dell_support() {
        let r = classify(
            "PowerEdge R760 is down, the idrac is unresponsive and the perc shows a failed VD",
        );
        assert_eq!(
            r.category, "dell_support",
            "a case naming Dell hardware lines must reach the subject-matter pool. \
             Got {:?} with keywords {:?}",
            r.category, r.matched_keywords
        );
        assert!(r.matched_keywords.iter().any(|k| k == "idrac"));
        assert!(r.matched_keywords.iter().any(|k| k == "poweredge"));
    }

    /// The pool does NOT SWALLOW the engagement categories.
    ///
    /// **This pin discriminates on a TIE-BREAK, and that is deliberate.** An
    /// earlier version of it asserted only that generic infrastructure language
    /// lands on `infrastructure` — and it PASSED with `server`/`network`/
    /// `storage` planted into the pool's own lexicon. The reason: the classifier
    /// returns the highest-scoring category, and a plant that adds three words
    /// to a pool that already matches the text produces a 3-vs-3 TIE, which the
    /// stable sort resolves toward the earlier category. The outcome was
    /// unchanged, so the pin proved nothing.
    ///
    /// The property that actually distinguishes the pool is therefore stronger
    /// and stated directly: **the pool must not hold a generic-infrastructure
    /// word at all.** That is a structural property, it cannot be defeated by a
    /// tie, and it is what stops the pool quietly re-routing every non-Dell case.
    #[test]
    fn the_dell_pool_holds_no_generic_infrastructure_word() {
        const GENERIC: &[&str] = &[
            "server", "network", "storage", "backup", "linux", "database", "deploy",
        ];
        let pool: Vec<&str> = LEXICON[CATEGORIES
            .iter()
            .position(|c| *c == "dell_support")
            .expect("the pool is a scored category")]
        .to_vec();
        for g in GENERIC {
            assert!(
                !pool.contains(g),
                "the dell_support pool must not contain {g:?}. `infrastructure` owns the \
                 generic vocabulary: a word in both lexicons makes the two categories tie, \
                 and the winner is then decided by sort order rather than by meaning. \
                 Pool currently: {pool:?}"
            );
        }
    }

    /// And the behavioural twin: a case whose ONLY signal is generic
    /// infrastructure language must reach `infrastructure`, not the pool. This
    /// is the discrimination the tie-proof above makes structural — with the
    /// pool holding no generic word, the two categories cannot tie on this text.
    #[test]
    fn dell_support_does_not_swallow_generic_infrastructure() {
        // No Dell product line named. This must stay `infrastructure`.
        let r = classify("The server network storage backup needs attention");
        assert_eq!(
            r.category, "infrastructure",
            "generic infrastructure language must not be captured by the Dell pool — \
             the pool is for the SUBJECT, not for hardware-shaped words. \
             Got {:?} with keywords {:?}",
            r.category, r.matched_keywords
        );
        // Prove the pool contributed nothing at all, rather than merely losing.
        assert!(
            !r.matched_keywords
                .iter()
                .any(|k| k == "poweredge" || k == "idrac"),
            "the Dell lexicon fired on a text with no Dell vocabulary — it must be inert here"
        );
    }

    /// The eight engagement categories are UNAFFECTED by the ninth's arrival.
    /// Derived from `CATEGORIES` rather than hand-typed, so it cannot rot: it
    /// asserts every non-`dell_support` category is still present and still
    /// ordered as before.
    #[test]
    fn dell_support_did_not_displace_any_existing_category() {
        let cats = CATEGORIES;
        assert_eq!(
            cats.len(),
            9,
            "nine categories: the original eight plus the dell_support pool"
        );
        // The load-bearing slice is [0..7]: those seven are the SCORED ones,
        // and `LEXICON` mirrors exactly this range in order. `general` is the
        // fallback and is never scored, so its position is a separate concern
        // (asserted below). Inserting the pool anywhere inside [0..7] would
        // re-pair every keyword with the wrong category.
        assert_eq!(
            &cats[..7],
            &[
                "technology",
                "business_process",
                "compliance",
                "finance",
                "vendor",
                "assessment",
                "infrastructure"
            ],
            "the first SEVEN must keep their exact indices — LEXICON mirrors \
             CATEGORIES[0..7] in order. Inserting the pool inside that range \
             would silently re-pair every keyword with the wrong category."
        );
        assert_eq!(
            cats[7], "dell_support",
            "the pool occupies the eighth slot, immediately before the fallback"
        );
        assert_eq!(
            cats[cats.len() - 1],
            "general",
            "`general` stays LAST so it remains the fallback: an empty match must \
             still land on general, never on the pool."
        );
    }

    /// No lexicon may name the same word twice. `keyword_fires` is whole-token
    /// equality, so a duplicated word makes ONE token vote TWICE — inflating the
    /// category's share of fired keywords and therefore its confidence, on no
    /// additional evidence. That is precisely the fabricated-score defect the
    /// substring-matching fix above was made to close, reintroduced by a
    /// copy-paste slip: `poweredge` shipped twice in the dell_support pool.
    #[test]
    fn no_lexicon_names_the_same_word_twice() {
        for (i, cat) in CATEGORIES
            .iter()
            .enumerate()
            .filter(|(_, c)| **c != "general")
        {
            let words = LEXICON[i];
            let mut seen = std::collections::BTreeSet::new();
            let dupes: Vec<&str> = words.iter().copied().filter(|w| !seen.insert(*w)).collect();
            assert!(
                dupes.is_empty(),
                "the {cat} lexicon names {dupes:?} more than once. A duplicate makes one \
                 token vote twice, inflating that category's share of fired keywords and \
                 therefore its confidence on no extra evidence — the same fabricated-score \
                 class as the substring-matching defect."
            );
        }
    }
    /// The scored list is DERIVED, and `LEXICON` must cover it exactly. A
    /// category with no lexicon entry would index out of bounds; a lexicon entry
    /// with no category would be scored under a name nothing emits.
    #[test]
    fn the_lexicon_covers_every_scored_category_exactly() {
        let scored: Vec<&str> = scored_categories().collect();
        assert_eq!(
            LEXICON.len(),
            scored.len(),
            "LEXICON has {} entries for {} scored categories — the two must be \
             equal or `classify` reads past the end (or scores under a name \
             nothing emits)",
            LEXICON.len(),
            scored.len()
        );
        assert_eq!(LEXICON.len() + 1, CATEGORIES.len());
    }

    /// An UNMATCHED text still falls back to `general`, never to the pool. The
    /// pool competes on keyword share; with nothing fired there is no share.
    #[test]
    fn an_unmatched_text_still_falls_back_to_general() {
        let r = classify("please advise on your preferred meeting times");
        assert_eq!(
            r.category, "general",
            "no keyword fired, so nothing competes and the fallback must win. \
             Got {:?}",
            r.category
        );
        assert_eq!(r.evidence_count, 0);
        assert!(r.confidence == 0.0);
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
