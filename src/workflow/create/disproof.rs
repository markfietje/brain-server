//! The disproof condition: **what would refute this claim, stated at write
//! time and evaluated at read time.**
//!
//! # Why this module exists
//!
//! Two rounds deferred to each other for four releases, and **neither document
//! named the other's deferral**:
//!
//! * One earlier round refused to ship this field because *"a field nothing
//!   reads… a later round owns the reader."*
//! * That later round deferred its falsification scheduler because *"no
//!   disproof-condition representation at all… **a design decision with a
//!   named owner, not an implementation task**"*.
//!
//! **Both deferrals named the same absent thing.** The owner has since been
//! named — the create-core plan carries the design — so **the stated blocker is
//! discharged** and this module is that work.
//!
//! # The rule this module installs
//!
//! Per `ASSESSMENT_SWE_PROOF_2609.21190` §1, a natural-language disproof
//! condition is a **weak check that accepts a great deal a targeted adversary can
//! then refute** — the paper carries a confirmed violation on **32%** of
//! resolving submissions. So there are exactly **two** forms and no third:
//!
//! * **(A) [`DisproofForm::Evaluated`]** — a machine-checked predicate or byte
//!   range over stored text. The system **evaluates** it. Not a sentence it reads.
//! * **(B) [`DisproofForm::Audited`]** — prose, and it ships **with** its
//!   faithfulness audit, never with the intention of one.
//!
//! > **A prose condition with neither an evaluation nor an audit is refused at
//! > construction. Not warned — refused.**
//!
//! And per `I60.6`, **coverage is mandatory** alongside either form, and
//! partial coverage is a **first-class class**, not an exception.
//!
//! # Honest ceilings — stated, not buried
//!
//! * **This module decides nothing.** It validates and evaluates a condition; the
//!   *falsification scheduler* (`I57.4`) is the separate round that runs the
//!   evaluation on a cadence. **A condition recorded here is still unexercised
//!   until that scheduler exists, and this is stated rather than implied.**
//! * **An `Evaluated` condition over stored text proves nothing about the
//!   world's truth** — only that the stored bytes still satisfy the predicate the
//!   author wrote. It is a *consistency* check. That is what makes it mechanical,
//!   and it is why the adversarial half of the question (does the human verdict
//!   survive attack?) is **not** answered by this module.
//! * **`Audited` conditions carry no mechanical verdict at all.** They are carried
//!   so a reader can see that prose was admitted *with* an audit attached — the
//!   refusal is the check, and the audit is the thing that makes the admission
//!   honest.

use serde::{Deserialize, Serialize};

/// The two disproof forms. **There is no third**, and adding one is the failure
/// mode the assessment names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisproofForm {
    /// (A) Machine-checked: the system evaluates this against stored bytes.
    Evaluated,
    /// (B) Prose, shipped with its faithfulness audit. Never with the intention
    /// of one.
    Audited,
}

impl DisproofForm {
    pub fn as_str(&self) -> &'static str {
        match self {
            DisproofForm::Evaluated => "evaluated",
            DisproofForm::Audited => "audited",
        }
    }

    /// The closed persistence vocabulary. A form outside it is refused at
    /// construction rather than coerced.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "evaluated" => Ok(DisproofForm::Evaluated),
            "audited" => Ok(DisproofForm::Audited),
            other => Err(format!("DI_DISPROOF_FORM_UNKNOWN:{other}")),
        }
    }
}

/// The comparison an [`DisproofForm::Evaluated`] condition applies over a byte
/// range. **Closed vocabulary** — an unknown operator is refused, because a
/// condition whose operator cannot be parsed is a condition nothing evaluates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvaluatedOp {
    /// The cited text is **present** in the subject. This is the DISPROOF
    /// reading — the condition names what would refute the claim, so the claim
    /// is refuted exactly when the cited text shows up.
    Contains,
    /// The cited text is **absent** from the subject. The inverse of
    /// [`EvaluatedOp::Contains`]: the claim stands while the cited text is gone,
    /// so a claim that *relies* on text being gone is satisfied by its absence.
    Absent,
}

impl EvaluatedOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            EvaluatedOp::Contains => "contains",
            EvaluatedOp::Absent => "absent",
        }
    }
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "contains" => Ok(EvaluatedOp::Contains),
            "absent" => Ok(EvaluatedOp::Absent),
            other => Err(format!("DI_DISPROOF_OP_UNKNOWN:{other}")),
        }
    }
}

/// Bounds law, shared by every byte range. Kept as one pair so a new surface
/// cannot invent a looser bound.
pub const MAX_DISPROOF_BODY_BYTES: usize = 2_048;
pub const MAX_DISPROOF_CITATION_BYTES: usize = 256;
pub const MAX_DISPROOF_SCOPE_BYTES: usize = 256;
/// The coverage declaration is mandatory, so it cannot be unbounded either.
pub const MAX_DISPROOF_COVERAGE_ITEMS: usize = 32;

/// A disproof condition as carried on a claim.
///
/// **Legacy rows carry `None`.** That is stamp-blind by declaration, following
/// the `content_owner_stamp` precedent — a condition was never written for them
/// and a NULL is not a claim that none was required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisproofCondition {
    pub form: DisproofForm,
    /// (A) the predicate body, or (B) the prose. Bounded either way.
    pub body: String,
    /// (A) only: the operator. `None` for (B) — prose has no operator, and
    /// storing one would be a form smuggling an evaluation it never got.
    pub op: Option<EvaluatedOp>,
    /// (A) only: the byte range the predicate applies to. `None` for (B).
    pub citation: Option<String>,
    /// (A) only: the machine-verifiable scope the condition covers. `None` for
    /// (B), which is scoped by its audit instead.
    pub scope: Option<String>,
    /// Mandatory alongside either form. **Partial coverage is a first-class
    /// class**, not an exception — so this is a list of what is covered, and an
    /// empty list is refused rather than defaulted.
    pub coverage: Vec<String>,
    /// (B) only: the faithfulness audit that makes the prose admission honest.
    /// **Mandatory for (B)** — a prose condition with neither an evaluation nor
    /// an audit is refused, and this is the field that carries the audit.
    pub audit_ref: Option<String>,
}

/// The seven persisted columns, exactly as a row carries them.
///
/// This is the wire shape between the table and [`DisproofCondition`], and it is
/// **deliberately not the condition**: it is all-`Option` because SQL `NULL` and
/// the empty string are different things, and conflating them is how a
/// half-written condition starts reading as a legacy row.
///
/// `coverage` is a JSON array rather than a delimited string, because a
/// coverage item is free prose and a delimiter would have to be escaped in a
/// column whose only other content is a form spelling. It is
/// `NOT NULL DEFAULT '[]'`, so **`'[]'` and `NULL` both mean "no coverage
/// list"** and both are treated as empty on the way back in.
///
/// # `scope` now HAS a column — the seventh, and the round that added it
///
/// [`DisproofCondition`] has **seven** fields. Schema 1.32.22 added **six**
/// columns: `disproof_form`, `disproof_body`, `disproof_op`,
/// `disproof_citation`, `disproof_coverage`, `disproof_audit_ref`. **`scope`
/// had no column**, and because `DisproofForm::Evaluated` **requires** a scope,
/// the representation could not persist the very form that makes a condition
/// machine-checkable. The writer refused rather than dropping the field, which
/// is how the ceiling was found: a refusal with a named cause instead of a
/// corrupt row.
///
/// Schema **1.32.23** adds `disproof_scope` as the seventh additive column, so
/// the two shapes are now the same width and both forms round-trip. The refusal
/// is gone because the thing it refused is now storable — see
/// [`DisproofCondition::to_columns`], which no longer returns a `Result`,
/// because there is nothing left for it to refuse.
///
/// A row written before 1.32.23 carries `NULL` there, which is the
/// stamp-blind legacy story every other column already tells. A row that names
/// `evaluated` and has `NULL` scope is therefore **damaged**, not legacy, and
/// is refused by the constructor — the `Ok(None)`-laundering problem the
/// read-back exists to prevent applies to this column like any other.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisproofColumns {
    pub form: Option<String>,
    pub body: Option<String>,
    pub op: Option<String>,
    pub citation: Option<String>,
    pub scope: Option<String>,
    pub coverage: Option<String>,
    pub audit_ref: Option<String>,
}

impl DisproofCondition {
    /// Construct with validation. **This is the only constructor**, which is why
    /// the refusal cannot be bypassed by building the struct and inserting it.
    pub fn new(
        form: DisproofForm,
        body: impl Into<String>,
        op: Option<EvaluatedOp>,
        citation: Option<String>,
        scope: Option<String>,
        coverage: Vec<String>,
        audit_ref: Option<String>,
    ) -> Result<Self, String> {
        let c = DisproofCondition {
            form,
            body: body.into(),
            op,
            citation,
            scope,
            coverage,
            audit_ref,
        };
        c.validate()?;
        Ok(c)
    }

    /// **The refusal the assessment requires**, checked before anything else so
    /// the failure names the real cause.
    pub fn validate(&self) -> Result<(), String> {
        if self.body.trim().is_empty() {
            return Err("DI_DISPROOF_EMPTY".into());
        }
        if self.body.len() > MAX_DISPROOF_BODY_BYTES {
            return Err(format!("DI_DISPROOF_BODY_TOO_LONG:{}", self.body.len()));
        }
        if let Some(op) = self.op
            && op.as_str().len() > MAX_DISPROOF_BODY_BYTES
        {
            return Err("DI_DISPROOF_OP_TOO_LONG".into());
        }
        if let Some(citation) = &self.citation
            && (citation.is_empty() || citation.len() > MAX_DISPROOF_CITATION_BYTES)
        {
            return Err("DI_DISPROOF_CITATION_BAD".into());
        }
        if let Some(scope) = &self.scope
            && (scope.is_empty() || scope.len() > MAX_DISPROOF_SCOPE_BYTES)
        {
            return Err("DI_DISPROOF_SCOPE_BAD".into());
        }
        // `I60.6`: coverage is MANDATORY alongside either form. Partial coverage
        // is a first-class class; *no* coverage is a refusal.
        if self.coverage.is_empty() {
            return Err("DI_DISPROOF_COVERAGE_REQUIRED".into());
        }
        if self.coverage.len() > MAX_DISPROOF_COVERAGE_ITEMS {
            return Err("DI_DISPROOF_COVERAGE_TOO_LARGE".into());
        }
        if self.coverage.iter().any(|c| c.trim().is_empty()) {
            return Err("DI_DISPROOF_COVERAGE_ITEM_EMPTY".into());
        }
        match self.form {
            DisproofForm::Evaluated => {
                if self.op.is_none() {
                    return Err("DI_DISPROOF_EVALUATED_NEEDS_OP".into());
                }
                if self.citation.is_none() {
                    return Err("DI_DISPROOF_EVALUATED_NEEDS_CITATION".into());
                }
                if self.scope.is_none() {
                    return Err("DI_DISPROOF_EVALUATED_NEEDS_SCOPE".into());
                }
            }
            DisproofForm::Audited => {
                if self.audit_ref.is_none() {
                    return Err("DI_DISPROOF_PROSE_NEEDS_AUDIT".into());
                }
            }
        }
        Ok(())
    }

    /// Evaluate against the subject text. **`None` for `Audited`** — and
    /// returning `None` is not a pass and not a fail; it is the honest
    /// "no mechanical verdict exists", which is exactly what a prose condition
    /// is.
    ///
    /// # What the returned `bool` MEANS — read this before using it
    ///
    /// `Some(true)` means **the condition is MET**: the disproof the author
    /// specified has been observed, so the claim is **REFUTED**. `Some(false)`
    /// means the condition is unmet and the claim **stands**.
    ///
    /// The polarity is the easy thing to get backwards, and getting it backwards
    /// produces a module that compiles, evaluates, and inverts every verdict. So
    /// it is stated here, asserted in both directions in the tests, and named
    /// [`DisproofVerdict::Refuted`] rather than left to a reader to infer.
    pub fn evaluate(&self, subject: &str) -> Option<bool> {
        if self.form != DisproofForm::Evaluated {
            return None;
        }
        let citation = self.citation.as_deref()?;
        match self.op? {
            // MET ⇔ the cited text is present. The disproof reading: the author
            // named what would refute the claim, so seeing it refutes.
            EvaluatedOp::Contains => Some(subject.contains(citation)),
            // MET ⇔ the cited text is absent. The inverse operator, and genuinely
            // a different condition — not a relabelling of the one above.
            EvaluatedOp::Absent => Some(!subject.contains(citation)),
        }
    }

    /// The verdict triple. `Satisfied` is only reachable for `Evaluated`; an
    /// `Audited` condition reports `NoVerdict` and is never silently green.
    ///
    /// # The mapping is about the CLAIM, not about the condition
    ///
    /// [`DisproofCondition::evaluate`] answers *"was the disproof observed?"*.
    /// This maps that to **the claim's standing**, which is what a caller wants
    /// and what the variant names say:
    ///
    /// * disproof observed (`Some(true)`) → [`DisproofVerdict::Refuted`]
    /// * disproof not observed (`Some(false)`) → [`DisproofVerdict::Satisfied`]
    ///
    /// Mapping it the other way — "the condition was satisfied" → `Satisfied` —
    /// is the bug this module first shipped with: a claim whose disproof was
    /// confirmed would have reported itself as **fine**. The polarity is
    /// asserted in both directions, under both operators, in the tests.
    pub fn verdict(&self, subject: &str) -> DisproofVerdict {
        match self.evaluate(subject) {
            Some(true) => DisproofVerdict::Refuted,
            Some(false) => DisproofVerdict::Satisfied,
            None => DisproofVerdict::NoVerdict {
                reason: "DI_DISPROOF_NO_MECHANICAL_VERDICT".into(),
            },
        }
    }

    /// **The fail-closed read-back.** Reconstruct a condition from a stored row.
    ///
    /// # The distinction this function exists to keep
    ///
    /// There are three outcomes and they are NOT two:
    ///
    /// | stored shape | result |
    /// |---|---|
    /// | every column empty | `Ok(None)` — a row that **predates the field** |
    /// | form present, reconstructs | `Ok(Some(c))` |
    /// | form present, does NOT reconstruct | **`Err`** |
    ///
    /// `Ok(None)` means *"no condition was ever written here"* and nothing else.
    /// **A corrupt row must never report `Ok(None)`**, because the caller cannot
    /// tell a claim that predates the field from a claim whose condition was
    /// damaged — and the first is exempt from evaluation while the second is
    /// exactly the claim that should be looked at hardest. Widening `None` to
    /// cover "unreadable" is the single defect this function is written to
    /// prevent, and the corrupt tests below pin each shape that would cause it.
    ///
    /// The refusal is **not** a second validation site: reconstruction builds
    /// through [`DisproofCondition::new`], the one constructor, so the
    /// admissibility laws have exactly one enforcement point and a row that
    /// reconstructs into something inadmissible is refused by the same rules
    /// that refused it at write time.
    pub fn from_columns(cols: &DisproofColumns) -> Result<Option<Self>, String> {
        let blank = |v: &Option<String>| v.as_deref().is_none_or(str::is_empty);
        let coverage_blank = cols
            .coverage
            .as_deref()
            .is_none_or(|c| c.is_empty() || c.trim() == "[]");

        // Legacy is a claim about EVERY column, not about one. A row with a
        // body and no form is a damaged row, not a pre-field row. `scope`
        // takes part like the rest: a row carrying a scope and nothing else is
        // a row that CLAIMED a condition, and reporting it `None` would exempt
        // it from evaluation for the wrong reason.
        let legacy = blank(&cols.form)
            && blank(&cols.body)
            && blank(&cols.op)
            && blank(&cols.citation)
            && blank(&cols.scope)
            && blank(&cols.audit_ref)
            && coverage_blank;
        if legacy {
            return Ok(None);
        }

        // Past this point the row CLAIMS a condition, so failing to rebuild one
        // is an error. Reporting `None` here would launder a damaged row as a
        // legacy one, which is the laundering this function refuses to do.
        let form_raw = cols
            .form
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or("DI_DISPROOF_ROW_FORM_MISSING")?;
        let form = DisproofForm::parse(form_raw)?;

        let op = cols
            .op
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(EvaluatedOp::parse)
            .transpose()?;

        let coverage = match cols.coverage.as_deref() {
            None | Some("") => Vec::new(),
            Some(raw) => decode_coverage(raw)?,
        };

        // Goes through the ONE constructor, so a row that cannot satisfy the
        // admissibility laws is refused here by the same laws that refused it
        // at construction time — and its error is returned, not swallowed.
        //
        // `scope` is read from its own column (schema 1.32.23). A row naming
        // `evaluated` with `NULL` scope is refused by the constructor, which is
        // the correct outcome for a row claiming a form it does not carry: the
        // scope is what makes the condition machine-checkable, and a condition
        // without it is not a lesser condition, it is no condition.
        let built = DisproofCondition::new(
            form,
            cols.body.as_deref().unwrap_or_default(),
            op,
            cols.citation.clone().filter(|s| !s.is_empty()),
            cols.scope.clone().filter(|s| !s.is_empty()),
            coverage,
            cols.audit_ref.clone().filter(|s| !s.is_empty()),
        )?;
        Ok(Some(built))
    }

    /// **The write side.** Serialise into the seven persisted columns.
    ///
    /// # Why this no longer returns a `Result`
    ///
    /// It used to, and the `Err` existed for exactly one reason: the table had
    /// six disproof columns and the condition has seven fields, so `scope` had
    /// nowhere to go. Because `DisproofForm::Evaluated` **requires** a scope,
    /// refusing to store it would have meant storing an `Evaluated` condition
    /// stripped of the field that makes it machine-checkable — a row that reads
    /// back as a condition nobody wrote and cannot be rebuilt.
    ///
    /// Schema 1.32.23 added `disproof_scope`, so the two shapes are the same
    /// width and **there is nothing left to refuse here**. A `Result` with no
    /// error path would be a second, permanently-unreachable failure mode
    /// dressed as a guard, so the signature says what it means: infallible.
    ///
    /// Admissibility is still decided in exactly one place — the constructor,
    /// which validates — and nothing can hand this function a condition that
    /// constructor did not admit. A writer that validated again here would be a
    /// second law that could drift from the first.
    pub fn to_columns(&self) -> DisproofColumns {
        DisproofColumns {
            form: Some(self.form.as_str().to_string()),
            body: Some(self.body.clone()),
            op: self.op.map(|o| o.as_str().to_string()),
            citation: self.citation.clone(),
            scope: self.scope.clone(),
            coverage: Some(encode_coverage(&self.coverage)),
            audit_ref: self.audit_ref.clone(),
        }
    }
}

/// Serialise the coverage list. A `Vec<String>` of JSON strings is the same
/// shape [`decode_coverage`] reads, which is what makes the round-trip exact
/// rather than approximately symmetric.
fn encode_coverage(items: &[String]) -> String {
    serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string())
}

/// Decode the coverage column. Bounded by the same item cap the constructor
/// enforces, and a column that is not a JSON array of strings is **refused**
/// rather than coerced to an empty list — an empty list would then be refused
/// for emptiness, which names the wrong cause.
fn decode_coverage(raw: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("DI_DISPROOF_ROW_COVERAGE_NOT_JSON:{e}"))?;
    let arr = value
        .as_array()
        .ok_or_else(|| "DI_DISPROOF_ROW_COVERAGE_NOT_ARRAY".to_string())?;
    let mut out = Vec::new();
    for v in arr.iter().take(MAX_DISPROOF_COVERAGE_ITEMS + 1) {
        let s = v
            .as_str()
            .ok_or_else(|| "DI_DISPROOF_ROW_COVERAGE_ITEM_NOT_STRING".to_string())?;
        out.push(s.to_string());
    }
    if out.len() > MAX_DISPROOF_COVERAGE_ITEMS {
        return Err(format!("DI_DISPROOF_ROW_COVERAGE_TOO_LARGE:{}", out.len()));
    }
    Ok(out)
}

/// What evaluating a condition can conclude. **`NoVerdict` is a distinct state
/// and is never a pass** — the same discipline the admission-gate suite installs
/// one layer down, applied here so the two are not different laws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisproofVerdict {
    Satisfied,
    Refuted,
    NoVerdict { reason: String },
}

impl DisproofVerdict {
    pub fn is_satisfied(&self) -> bool {
        matches!(self, DisproofVerdict::Satisfied)
    }
    pub fn is_refuted(&self) -> bool {
        matches!(self, DisproofVerdict::Refuted)
    }
    /// A prose condition is not green. This predicate exists so a reader
    /// cannot mistake `NoVerdict` for `Satisfied` by inattention.
    pub fn is_green(&self) -> bool {
        matches!(self, DisproofVerdict::Satisfied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluated() -> DisproofCondition {
        DisproofCondition::new(
            DisproofForm::Evaluated,
            "the payer on file matches the claim",
            Some(EvaluatedOp::Contains),
            Some("payer-verified".into()),
            Some("subject.payer_state".into()),
            vec!["payer_state".into()],
            None,
        )
        .expect("an evaluated condition with an op, citation, scope and coverage")
    }

    /// The same condition under a chosen operator, so a polarity assertion can
    /// flip the operator and hold everything else fixed. **A polarity asserted
    /// for one operator only is a polarity nobody has checked.**
    fn evaluated_with(op: EvaluatedOp) -> DisproofCondition {
        DisproofCondition::new(
            DisproofForm::Evaluated,
            "the payer on file matches the claim",
            Some(op),
            Some("payer-verified".into()),
            Some("subject.payer_state".into()),
            vec!["payer_state".into()],
            None,
        )
        .expect("an evaluated condition with an op, citation, scope and coverage")
    }

    fn audited() -> DisproofCondition {
        DisproofCondition::new(
            DisproofForm::Audited,
            "this claim is false if the member's consent was not recorded",
            None,
            None,
            None,
            vec!["consent".into()],
            Some("audit:care/consent-441".into()),
        )
        .expect("a prose condition shipped with its audit")
    }

    // ── the refusal the assessment requires ───────────────────────────────

    #[test]
    fn prose_without_an_audit_is_refused_not_warned() {
        let err = DisproofCondition::new(
            DisproofForm::Audited,
            "this is false if the member never consented",
            None,
            None,
            None,
            vec!["consent".into()],
            None, // ← no audit
        )
        .expect_err("a prose condition with neither evaluation nor audit must be refused");
        assert_eq!(err, "DI_DISPROOF_PROSE_NEEDS_AUDIT");
    }

    #[test]
    fn coverage_is_mandatory_for_both_forms() {
        for form in [DisproofForm::Evaluated, DisproofForm::Audited] {
            let err = DisproofCondition::new(
                form,
                "body",
                Some(EvaluatedOp::Contains),
                Some("x".into()),
                Some("s".into()),
                vec![], // ← no coverage
                Some("audit:x".into()),
            )
            .expect_err("coverage is mandatory alongside either form");
            assert_eq!(err, "DI_DISPROOF_COVERAGE_REQUIRED");
        }
    }

    #[test]
    fn evaluated_without_its_operator_is_refused() {
        let err = DisproofCondition::new(
            DisproofForm::Evaluated,
            "body",
            None, // ← no operator
            Some("x".into()),
            Some("s".into()),
            vec!["c".into()],
            None,
        )
        .expect_err("an evaluated condition with no operator is not machine-checkable");
        assert_eq!(err, "DI_DISPROOF_EVALUATED_NEEDS_OP");
    }

    #[test]
    fn an_empty_body_is_refused() {
        let err = DisproofCondition::new(
            DisproofForm::Evaluated,
            "   ",
            Some(EvaluatedOp::Contains),
            Some("x".into()),
            Some("s".into()),
            vec!["c".into()],
            None,
        )
        .expect_err("an empty body is not a condition");
        assert_eq!(err, "DI_DISPROOF_EMPTY");
    }

    #[test]
    fn the_form_and_op_vocabularies_are_closed() {
        assert_eq!(
            DisproofForm::parse("evaluated"),
            Ok(DisproofForm::Evaluated)
        );
        assert_eq!(DisproofForm::parse("audited"), Ok(DisproofForm::Audited));
        assert!(DisproofForm::parse("Evaluated").is_err(), "case-sensitive");
        assert!(DisproofForm::parse("prose").is_err(), "no third form");
        assert_eq!(EvaluatedOp::parse("contains"), Ok(EvaluatedOp::Contains));
        assert!(EvaluatedOp::parse("regex").is_err(), "no open vocabulary");
    }

    // ── evaluation ───────────────────────────────────────────────────────

    #[test]
    fn an_evaluated_condition_evaluates_and_its_polarity_is_explicit() {
        let c = evaluated();
        // `Contains` is the DISPROOF reading: the condition names what would
        // refute the claim, so seeing the cited text REFUTES the claim and not
        // seeing it leaves the claim standing. The polarity is asserted
        // explicitly because the inverted mapping also compiles, also
        // "evaluates", and silently inverts every verdict in the system.
        assert_eq!(
            c.verdict("payer-verified on file"),
            DisproofVerdict::Refuted,
            "the cited text is present, so the disproof condition is met and the claim is refuted"
        );
        assert_eq!(
            c.verdict("payer unverified"),
            DisproofVerdict::Satisfied,
            "the cited text is absent, so the disproof condition is unmet and the claim stands"
        );
        // And the polarity INVERTS under the inverse operator, on the SAME input.
        // This is the half a single-operator assertion cannot see — and it is
        // what the first draft of this test got wrong, by asserting both
        // operators produced identical verdicts, which would have made the
        // operator meaningless and the pin vacuous.
        assert_eq!(
            evaluated_with(EvaluatedOp::Absent).verdict("payer unverified"),
            DisproofVerdict::Refuted,
            "the inverse operator reads 'false if the text is gone', so absent refutes"
        );
        assert_eq!(
            evaluated_with(EvaluatedOp::Absent).verdict("payer-verified on file"),
            DisproofVerdict::Satisfied,
            "and on the same input the two operators disagree — which is the property under test"
        );
    }

    #[test]
    fn the_absent_op_is_the_inverse_and_says_so() {
        let c = DisproofCondition::new(
            DisproofForm::Evaluated,
            "body",
            Some(EvaluatedOp::Absent),
            Some("payer-verified".into()),
            Some("s".into()),
            vec!["c".into()],
            None,
        )
        .expect("valid");
        // `Absent` is the exact inverse of `Contains` on the same two inputs,
        // swapped. If either operator stops inverting, one of these flips.
        assert_eq!(
            c.verdict("payer unverified"),
            DisproofVerdict::Refuted,
            "under Absent, the cited text being gone is the disproof"
        );
        assert_eq!(
            c.verdict("payer-verified on file"),
            DisproofVerdict::Satisfied,
            "under Absent, the cited text being present means the claim stands"
        );
    }

    #[test]
    fn prose_evaluates_to_no_verdict_and_never_to_green() {
        let v = audited().verdict("anything at all");
        assert_eq!(
            v,
            DisproofVerdict::NoVerdict {
                reason: "DI_DISPROOF_NO_MECHANICAL_VERDICT".into()
            }
        );
        assert!(!v.is_green(), "a prose condition is never green");
        assert!(!v.is_satisfied());
        assert!(!v.is_refuted());
    }

    #[test]
    fn the_verdict_states_partition() {
        // NoVerdict must not be reachable through either green or red, so a
        // caller cannot read it as either by inattention.
        let s = DisproofVerdict::Satisfied;
        let r = DisproofVerdict::Refuted;
        let n = DisproofVerdict::NoVerdict { reason: "x".into() };
        assert!(s.is_green() && !s.is_refuted());
        assert!(r.is_refuted() && !r.is_green());
        assert!(!n.is_green() && !n.is_refuted());
    }

    #[test]
    fn legacy_claims_carry_no_condition_and_that_is_not_a_refutation() {
        // Replaced. The previous version of this test bound a literal `None` to
        // an `Option<DisproofCondition>` and asserted `is_none()` on it — a
        // literal compared against itself, which passes whether or not
        // `DisproofCondition` exists at all. It was a pin that could not fail,
        // which is worse than no pin.
        //
        // NOTE: the marker string the external guard in `tests/r50_create.rs`
        // greps for is deliberately NOT reproduced verbatim in this comment. A
        // guard that matches a string its own subject quotes is a guard that
        // fails on documentation, which is how a source pin becomes a pin that
        // cannot pass. Describe the defect; do not spell the needle.
        //
        // The real legacy claim is a claim about a STORED ROW, so this now goes
        // through the read-back against the column shape a pre-field row
        // actually carries: every column NULL, and coverage at its `'[]'`
        // default. `Ok(None)` here means "predates the field" and nothing else.
        for coverage in [Some("[]"), Some(""), None] {
            let cols = DisproofColumns {
                coverage: coverage.map(str::to_string),
                ..DisproofColumns::default()
            };
            assert_eq!(
                DisproofCondition::from_columns(&cols),
                Ok(None),
                "a row with no form, body, op, citation, scope or audit predates the field, and \
                 coverage {coverage:?} is the empty form of the mandatory declaration"
            );
        }

        // And the half of the story the tautology could not see: `None` is
        // reserved for "predates the field", so a row that carries ANY other
        // disproof byte is NOT legacy. A body alone is enough.
        let body_only = DisproofColumns {
            body: Some("a condition body".into()),
            coverage: Some("[]".into()),
            ..DisproofColumns::default()
        };
        assert!(
            DisproofCondition::from_columns(&body_only).is_err(),
            "a body with no form is a DAMAGED row, not a legacy one. Reporting None here would \
             exempt a damaged claim from evaluation — the laundering the read-back exists to stop"
        );

        // The seventh column is the one this round added, so it is the one whose
        // omission from the legacy conjunction would open the same hole silently:
        // a row carrying ONLY a scope names a machine-checkable extent with no
        // condition attached, and `Ok(None)` would exempt exactly that claim.
        let scope_only = DisproofColumns {
            scope: Some("claim.payer_state".into()),
            coverage: Some("[]".into()),
            ..DisproofColumns::default()
        };
        assert!(
            DisproofCondition::from_columns(&scope_only).is_err(),
            "a scope with no form is a DAMAGED row, not a legacy one. `scope` must take part in \
             the legacy determination like every other column, or the seventh column becomes the \
             one place `None` can still mean unreadable"
        );
    }

    /// The round-trip, in the pure layer, before any SQL is involved: a
    /// condition that survives its own column form byte for byte. If this fails,
    /// a database round-trip cannot be trusted either.
    ///
    /// **Both** forms are exercised, and that is the change schema 1.32.23
    /// bought: while `scope` had no column, `Evaluated` could not be
    /// serialised at all, so the only thing this pin could honestly cover was
    /// the prose form. A pin that covers half the representation is a pin that
    /// would have stayed green if the seventh column were wired backwards.
    #[test]
    fn a_condition_survives_its_own_column_form() {
        for c in [audited(), evaluated()] {
            let cols = c.to_columns();
            assert_eq!(
                DisproofCondition::from_columns(&cols),
                Ok(Some(c.clone())),
                "the columns must rebuild the exact condition — including which fields are None, \
                 because a form that materialises an absent citation is a form that invents an \
                 evaluation the author never wrote"
            );
        }
    }

    /// **The `scope` ceiling, INVERTED — the seventh column exists.**
    ///
    /// This pin asserted the opposite for a release: schema 1.32.22 added six
    /// disproof columns and [`DisproofCondition`] has seven fields, so `scope`
    /// had nowhere to go and every `Evaluated` condition — which REQUIRES a
    /// scope — was refused at the serialisation seam by a named refusal code.
    /// That refusal was correct then and is wrong now, so the pin was
    /// **inverted rather than deleted**: a replaced pin that disappears takes
    /// its coverage with it, and the coverage here is the load-bearing claim —
    /// that the form which makes a condition machine-checkable is storable, and
    /// stored byte for byte.
    ///
    /// It is still falsifiable, and the way it fails matters: if the column were
    /// dropped on the floor again, `to_columns` would either drop the field (and
    /// the round-trip below would rebuild a condition that is not the one
    /// written) or refuse. Both lose.
    #[test]
    fn an_evaluated_condition_is_stored_because_scope_has_a_column() {
        let c = evaluated();
        let cols = c.to_columns();
        assert_eq!(
            cols.scope.as_deref(),
            c.scope.as_deref(),
            "the scope must reach its own column verbatim — this is the field whose absence made \
             the form unpersistable, and a lossy copy of it would make the condition unfalsifiable"
        );
        assert_eq!(
            DisproofCondition::from_columns(&cols),
            Ok(Some(c.clone())),
            "an Evaluated condition must survive its own column form. It is the form the \
             constructor REQUIRES a scope for, so a round-trip that lost it would come back \
             refused — the corruption-at-the-write-seam this module refuses to manufacture"
        );
        // And the prose form still carries NO scope, so the two forms stay
        // distinguishable on disk rather than both writing something in column
        // seven.
        assert_eq!(
            audited().to_columns().scope,
            None,
            "prose is scoped by its audit, not by a machine-checkable scope; writing one here \
             would invent a field the author never set"
        );
    }

    /// A row that names `evaluated` but carries no scope is DAMAGED, and must be
    /// refused — never `Ok(None)`, which would exempt it from evaluation for the
    /// wrong reason.
    ///
    /// This shape is reachable, not hypothetical: it is exactly the row a writer
    /// that dropped `scope` on the floor would have written before schema
    /// 1.32.23, and exactly the row any hand-edited or restored legacy row can
    /// carry. The seventh column makes the form storable; it does not make a
    /// scope-free `evaluated` row admissible.
    #[test]
    fn a_row_claiming_evaluated_without_a_scope_cannot_rebuild_and_is_refused_not_legacy() {
        let cols = DisproofColumns {
            form: Some("evaluated".into()),
            body: Some("the payer matches".into()),
            op: Some("contains".into()),
            citation: Some("payer-verified".into()),
            scope: None,
            coverage: Some("[\"payer_state\"]".into()),
            audit_ref: None,
        };
        assert_eq!(
            DisproofCondition::from_columns(&cols),
            Err("DI_DISPROOF_EVALUATED_NEEDS_SCOPE".into()),
            "the row names a form whose required field it does not carry, so the honest \
             answer is a refusal — Ok(None) here would silently exempt a claim that claims a \
             machine-checked condition it does not have"
        );
        // The same row WITH the scope rebuilds, which is what makes the refusal
        // above about the missing field and not about the form being unreadable.
        let cols = DisproofColumns {
            scope: Some("claim.payer_state".into()),
            ..cols
        };
        assert!(
            DisproofCondition::from_columns(&cols).is_ok(),
            "adding the scope must be sufficient to rebuild — otherwise this pin would pass on a \
             row that is unreadable for some other reason"
        );
    }

    /// The empty-string/NULL conflation, which is the one place this design
    /// could quietly widen `None`. `op` is `None` for prose and absent in the
    /// row; if `from_columns` ever turned that into `Some("")` the constructor
    /// would refuse a prose condition it is supposed to accept.
    #[test]
    fn an_absent_field_round_trips_as_absent_not_as_empty_text() {
        let cols = audited().to_columns();
        assert_eq!(cols.op, None, "prose carries no operator");
        assert_eq!(cols.citation, None, "prose carries no byte range");
        assert_eq!(cols.scope, None, "prose carries no machine-checkable scope");
        assert_eq!(
            cols.audit_ref.as_deref(),
            Some("audit:care/consent-441"),
            "and its audit is the one field prose does carry"
        );
        assert_eq!(
            cols.coverage.as_deref(),
            Some("[\"consent\"]"),
            "coverage is declared, not inherited from the column default"
        );
    }

    /// `disproof_coverage` is `NOT NULL DEFAULT '[]'`, so a row written with no
    /// condition still carries a value there. The read-back must not mistake
    /// that default for a declared coverage list.
    #[test]
    fn the_coverage_default_is_not_a_declared_coverage_list() {
        // Form present, coverage at its default: the row claims a condition and
        // under-declares its coverage. Coverage is mandatory, so this is
        // refused — by the constructor, through the one seam.
        let cols = DisproofColumns {
            form: Some("audited".into()),
            body: Some("body".into()),
            op: None,
            citation: None,
            scope: None,
            coverage: Some("[]".into()),
            audit_ref: Some("audit:x".into()),
        };
        assert_eq!(
            DisproofCondition::from_columns(&cols),
            Err("DI_DISPROOF_COVERAGE_REQUIRED".into()),
            "an empty coverage list is a refusal, not a default to inherit"
        );
    }

    /// A coverage column that is not a JSON array must not be coerced to an
    /// empty list. Coercing would produce `DI_DISPROOF_COVERAGE_REQUIRED`,
    /// naming "you declared no coverage" for a row whose real fault is that its
    /// coverage column is unreadable.
    #[test]
    fn an_unreadable_coverage_column_is_named_as_such() {
        let cols = DisproofColumns {
            form: Some("audited".into()),
            body: Some("body".into()),
            op: None,
            citation: None,
            scope: None,
            coverage: Some("not json".into()),
            audit_ref: Some("audit:x".into()),
        };
        let err = DisproofCondition::from_columns(&cols)
            .expect_err("a coverage column that is not json cannot be read");
        assert!(
            err.starts_with("DI_DISPROOF_ROW_COVERAGE_NOT_JSON"),
            "the refusal must name the unreadable column, got {err}"
        );
    }
}
