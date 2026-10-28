//! The support verdict — the chat spine's cite-or-abstain gate (LL-04 §1.2).
//!
//! THE LAW THIS MODULE EXISTS TO HOLD: "Never fabricate a citation" stopped
//! being an unverified instruction and became a WIRE FACT. The model's typed
//! final (`BridgeEvent::Final{abstain, claims}`) names the grounding hits it
//! drew from; this module judges those names with PURE INDEX ARITHMETIC —
//! no LLM in the gate, no threshold, no model call. The kernel labels what
//! the model asserted; it never edits the model's prose.
//!
//! Rules (pinned, each one a test below):
//! - every `cites[i]` must index a hit the kernel ACTUALLY sent (the packed
//!   set); any out-of-range index flags that claim `unknown_refs`;
//! - `Abstained`  = `abstain ∧ claims empty` — the honest "I don't know";
//! - `Unsupported` = `unknown_refs > 0 ∨ (cited = 0 ∧ claims non-empty)` —
//!   the fabricated-citation label;
//! - `Partial`    = `uncited > 0` — some claims prove nothing;
//! - `Degraded`   = grounding was degraded (the recall error arm — the wire
//!   already said so; this carries it onto the turn of record).
//!
//! Precedence when several facts hold: Abstained, Unsupported, Partial,
//! Degraded, Supported. A fabrication label outranks a degraded-evidence
//! label — the stronger fact is the one a reader must not glide past; and
//! a fully-cited answer on degraded grounding is still `Degraded`, because
//! the turn's evidence never became sound.
//!
//! The verdict rides the wire as the `support` SSE event (after `final`,
//! exactly once) and persists on the `chat_turns` row (`support_decision`);
//! `Unsupported` turns are STORED FLAGGED — visible, never silently an
//! answer.

use crate::agentloop::bridge_afm::BridgeClaim;

/// The verdict's wire vocabulary — snake_case, like the grounding
/// decisions beside it. Stored verbatim in `chat_turns.support_decision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportDecision {
    Supported,
    Partial,
    Unsupported,
    Abstained,
    Degraded,
}

impl SupportDecision {
    /// The wire/storage name.
    pub fn as_str(&self) -> &'static str {
        match self {
            SupportDecision::Supported => "supported",
            SupportDecision::Partial => "partial",
            SupportDecision::Unsupported => "unsupported",
            SupportDecision::Abstained => "abstained",
            SupportDecision::Degraded => "degraded",
        }
    }

    /// Parse a stored/wire name back. Unknown values are storage damage and
    /// refuse rather than inventing a sixth decision.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "supported" => Some(SupportDecision::Supported),
            "partial" => Some(SupportDecision::Partial),
            "unsupported" => Some(SupportDecision::Unsupported),
            "abstained" => Some(SupportDecision::Abstained),
            "degraded" => Some(SupportDecision::Degraded),
            _ => None,
        }
    }
}

/// The support verdict over one turn's claims. Counts are CLAIM counts:
/// `cited` claims index at least one hit the kernel sent, `uncited` claims
/// index none, `unknown_refs` claims named an index outside the packed set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Support {
    pub cited: u16,
    pub uncited: u16,
    pub unknown_refs: u16,
    pub decision: SupportDecision,
}

/// The verdict, pure and total: any (abstain, claims, hit count, grounding
/// posture) maps to exactly one label. Totality is the gate's point — there
/// is no input shape that escapes labeling.
pub fn support_verdict(
    abstain: bool,
    claims: &[BridgeClaim],
    hit_count: usize,
    degraded_grounding: bool,
) -> Support {
    let mut cited: u16 = 0;
    let mut uncited: u16 = 0;
    let mut unknown_refs: u16 = 0;
    for claim in claims {
        let mut known = false;
        let mut named_unknown = false;
        for &index in &claim.cites {
            // Negative indices cannot name a hit either: usize cast folds
            // them far out of range, which is exactly how they must count.
            if (index as usize) < hit_count {
                known = true;
            } else {
                named_unknown = true;
            }
        }
        if named_unknown {
            unknown_refs += 1;
        }
        if known {
            cited += 1;
        } else {
            uncited += 1;
        }
    }
    let decision = if abstain && claims.is_empty() {
        SupportDecision::Abstained
    } else if unknown_refs > 0 || (cited == 0 && !claims.is_empty()) {
        SupportDecision::Unsupported
    } else if uncited > 0 {
        SupportDecision::Partial
    } else if degraded_grounding {
        SupportDecision::Degraded
    } else {
        SupportDecision::Supported
    };
    Support {
        cited,
        uncited,
        unknown_refs,
        decision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(text: &str, cites: &[i64]) -> BridgeClaim {
        BridgeClaim {
            text: text.to_owned(),
            cites: cites.to_vec(),
        }
    }

    /// THE FLAGSHIP RED-PROOF (LL-04 §5), FLIPPED FORM. The first form —
    /// `unknown_citation_index_passes_through_unflagged` — ran GREEN at
    /// e1a77296 before any LL-04 edit: a final whose only claim cited index
    /// 7 of the 5 hits the kernel sent left the wire with no claims, no
    /// abstain, no verdict. THE DEFECT. This is the assertion that replaces
    /// it: the same shape now flags the turn.
    #[test]
    fn unknown_citation_index_flags_the_turn() {
        let support = support_verdict(
            false,
            &[claim("Green tea contains EGCG [8].", &[7])],
            5,
            false,
        );
        assert_eq!(support.decision, SupportDecision::Unsupported);
        assert_eq!(support.unknown_refs, 1, "the out-of-range claim is counted");
        assert_eq!(
            support.uncited, 1,
            "a claim naming only unknowns is uncited"
        );
        assert_eq!(support.cited, 0);
    }

    /// THE §1.2 RULE SET, each rule its own arm. Totality is the gate's
    /// point: every shape lands on exactly one label.
    #[test]
    fn the_support_verdict_rules_hold() {
        // Abstained: abstain ∧ claims empty — on sound AND on degraded
        // grounding (the D1 abstain path emits exactly this shape).
        let a = support_verdict(true, &[], 0, false);
        assert_eq!(a.decision, SupportDecision::Abstained);
        assert_eq!(a.cited + a.uncited + a.unknown_refs, 0);
        let a_degraded = support_verdict(true, &[], 0, true);
        assert_eq!(a_degraded.decision, SupportDecision::Abstained);

        // Unsupported: cited == 0 with claims non-empty — a claim wall with
        // nothing behind it, even when every index is in range.
        let u = support_verdict(false, &[claim("x", &[]), claim("y", &[])], 5, false);
        assert_eq!(u.decision, SupportDecision::Unsupported);
        assert_eq!(u.uncited, 2);

        // Unsupported outranks Degraded: a fabrication on degraded
        // grounding is labeled by the STRONGER fact.
        let ud = support_verdict(false, &[claim("x", &[9])], 2, true);
        assert_eq!(ud.decision, SupportDecision::Unsupported);

        // Partial: uncited > 0 while at least one claim holds.
        let p = support_verdict(
            false,
            &[claim("grounded", &[0]), claim("floating", &[])],
            3,
            false,
        );
        assert_eq!(p.decision, SupportDecision::Partial);
        assert_eq!(p.cited, 1);
        assert_eq!(p.uncited, 1);

        // Degraded: grounding was degraded and nothing stronger applies —
        // even a fully-cited answer stays Degraded (the evidence never
        // became sound).
        let d = support_verdict(false, &[], 0, true);
        assert_eq!(d.decision, SupportDecision::Degraded);
        let d_cited = support_verdict(false, &[claim("g", &[0])], 1, true);
        assert_eq!(d_cited.decision, SupportDecision::Degraded);

        // Supported: claims non-empty, every claim in range.
        let s = support_verdict(false, &[claim("a", &[0]), claim("b", &[2, 4])], 5, false);
        assert_eq!(s.decision, SupportDecision::Supported);
        assert_eq!(s.cited, 2);

        // A claim citing NOTHING indexes nothing: uncited, never a crash —
        // and an empty claims list with no abstain is a Degraded-shaped
        // turn only when grounding was degraded; on sound grounding it is
        // the Unsupported wall (cited == 0 with claims non-empty is the
        // rule; zero CLAIMS is labeled by the grounding posture).
        let zero = support_verdict(false, &[], 5, false);
        assert_eq!(zero.decision, SupportDecision::Supported);
    }

    /// THE WIRE/STORAGE VOCABULARY round-trips, and an unknown value is
    /// storage damage, not a sixth decision.
    #[test]
    fn the_decision_names_round_trip_and_refuse_unknowns() {
        for decision in [
            SupportDecision::Supported,
            SupportDecision::Partial,
            SupportDecision::Unsupported,
            SupportDecision::Abstained,
            SupportDecision::Degraded,
        ] {
            assert_eq!(
                SupportDecision::parse(decision.as_str()),
                Some(decision),
                "{} round-trips",
                decision.as_str()
            );
        }
        assert_eq!(SupportDecision::parse("hallucinated"), None);
        assert_eq!(SupportDecision::parse(""), None);
        assert_eq!(
            SupportDecision::parse("Supported"),
            None,
            "the vocabulary is lowercase snake_case"
        );
    }
}
