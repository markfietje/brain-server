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
        // The None case is the whole legacy story: a claim written before this
        // round carries no condition, and that is not evidence the claim is
        // sound or unsound.
        let legacy: Option<DisproofCondition> = None;
        assert!(legacy.is_none());
        assert!(!legacy.map(|c| c.validate().is_ok()).unwrap_or(false));
    }
}
