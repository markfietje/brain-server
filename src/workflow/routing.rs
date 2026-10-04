//! The routing core: which queue owns a case, and which agents may be offered it.
//!
//! Two pure functions, no I/O, no clock, no daemon, no model call. Both are
//! total: every input produces an output, and no input can widen authority.
//!
//! ## `route_to_queue` reads the class. It never reads a number.
//!
//! The classifier's contribution to routing is its **class**, not its score. A
//! confidence number is accepted here and then discarded on purpose — see
//! [`route_to_queue`]. A case whose queue is not known routes to the escalation queue,
//! and **never** to the nearest known queue: inventing a destination nobody
//! declared is worse than admitting the case needs a human.
//!
//! ## The queue vocabulary is a parameter, not a constant here
//!
//! The declared queue ids live with the taxonomy that declares them, in the
//! consultancy repository, which the server does not read. A second copy in this
//! crate would be exactly the hand-typed list that had to be removed from that
//! generator — a copy that can fall behind its source with nothing failing. So
//! the caller supplies the vocabulary it routes against, and this crate holds
//! no list of its own.
//!
//! The one queue id written literally here is the escalation target, because it
//! is the destination the whole system agrees on: an outage, security, legal,
//! billing, or P1/P2 case is always human, whatever the class says.
//!
//! ## Offers, never assignments
//!
//! [`select_assignee`] returns [`Offer`]s. That type has no field naming an
//! assignee, no `assign`/`commit`/`accept` method, and no writer behind it, so
//! an assignment is not something this module can express. The knowledge ring's
//! law applied one axis over: the machine may propose who is free; it may not
//! decide who works.

use crate::workflow::crew::{ACTIVITY_KINDS, CrewMember};
use crate::workflow::shifts::{Shift, active_shift};

/// The destination every unknown-queue case lands on, always human.
pub const ESCALATION_QUEUE: &str = "Q-OPS-ESCALATION";

/// The activity kinds that count as available for new work.
///
/// The closed presence vocabulary has four members; two of them are free. An
/// agent who is `cranking` or `reviewing` is mid-act and is not offered work.
const AVAILABLE_KINDS: [&str; 2] = ["idle", "channel"];

/// Why a case could not be routed to a known queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalateReason {
    /// Nothing resolved a queue for this case.
    NoCandidate,
    /// Something proposed a queue, and the declared vocabulary does not have it.
    UndeclaredQueue,
}

/// Where a case goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteOutcome {
    /// A declared queue owns the case. `class` is the classifier's verdict,
    /// carried so the receipt says what was routed and not only where to.
    Routed { class: RoutingClass, queue: String },
    /// The escalation queue owns the case. There is no third outcome, and
    /// `escalated` is never a way to keep searching.
    Escalated {
        queue: &'static str,
        reason: EscalateReason,
    },
}

impl RouteOutcome {
    /// The queue that owns the case, whichever outcome this is.
    pub fn queue(&self) -> &str {
        match self {
            Self::Routed { queue, .. } => queue,
            Self::Escalated { queue, .. } => queue,
        }
    }

    /// Whether this case went to the escalation queue.
    pub fn is_escalated(&self) -> bool {
        matches!(self, Self::Escalated { .. })
    }
}

/// The classifier's contribution to routing: its class, and nothing else.
///
/// The variants mirror [`crate::procedural::CATEGORIES`] exactly, and a pin
/// holds the two vocabularies equal — a routing axis that drifts from the
/// classifier that feeds it routes into a vocabulary nothing emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingClass {
    Technology,
    BusinessProcess,
    Compliance,
    Finance,
    Vendor,
    Assessment,
    Infrastructure,
    /// The SUBJECT-MATTER pool. Named vendor-neutrally here; the wire label is
    /// [`crate::procedural::subject_matter_label`], so this tree carries no
    /// engagement name while a deployment keeps the label it persisted.
    SubjectMatter,
    General,
}

impl RoutingClass {
    /// Every class, in the classifier's order. Pinned equal to
    /// [`crate::procedural::CATEGORIES`] so the routing axis cannot drift from
    /// the classifier that feeds it.
    pub const ALL: [Self; 9] = [
        Self::Technology,
        Self::BusinessProcess,
        Self::Compliance,
        Self::Finance,
        Self::Vendor,
        Self::Assessment,
        Self::Infrastructure,
        Self::SubjectMatter,
        Self::General,
    ];

    /// The classifier's own name for this class.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Technology => "technology",
            Self::BusinessProcess => "business_process",
            Self::Compliance => "compliance",
            Self::Finance => "finance",
            Self::Vendor => "vendor",
            Self::Assessment => "assessment",
            Self::Infrastructure => "infrastructure",
            Self::SubjectMatter => crate::procedural::subject_matter_label(),
            Self::General => "general",
        }
    }

    /// Resolve a classifier label to the class that carries it.
    ///
    /// **The vocabulary is searched, not typed.** A `match` over label literals
    /// would be a second copy of the classifier's vocabulary, free to drift green
    /// beside [`Self::ALL`] — and [`Self::ALL`] is already pinned equal to
    /// [`crate::procedural::CATEGORIES`], so a search inherits that pin instead of
    /// competing with it.
    ///
    /// An unrecognised label is [`None`], never a guess at the nearest neighbour. This
    /// type has no absence class, so "the classifier did not say" has no value here
    /// that could be routed: a caller holding no class must refuse rather than
    /// invent one, and a caller must not be handed a class the classifier never
    /// emitted. There is deliberately no fallback arm.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.as_str() == label)
    }
}

/// Route a case to its owning queue.
///
/// `class` is the classifier's verdict and is carried for the caller's receipt;
/// routing itself is decided by whether `proposed` is in `declared`.
///
/// **`confidence` is accepted and discarded.** It is a parameter so that a
/// caller holding one is not tempted to invent a bypass, and it is bound to `_`
/// on the first line so no branch can read it. There is deliberately no code
/// path on which a number — however high — reaches the escalation queue's
/// sibling outcomes. Confidence is a *quality* signal; whether a destination
/// exists is a *fact* about the vocabulary, and a quality number cannot make an
/// undeclared destination declared.
///
/// `declared` is the queue vocabulary the caller routes against. A `proposed`
/// outside it escalates: no prefix match, no edit distance, no "closest known
/// queue". The set is exact or the case is escalated.
pub fn route_to_queue(
    class: RoutingClass,
    proposed: Option<&str>,
    declared: &[&str],
    confidence: Option<i32>,
) -> RouteOutcome {
    // `confidence` is read once and discarded — that binding IS the control.
    // `class` is carried: it is the receipt of what was routed.
    let _ = confidence;
    match proposed {
        None => RouteOutcome::Escalated {
            queue: ESCALATION_QUEUE,
            reason: EscalateReason::NoCandidate,
        },
        Some(q) if declared.contains(&q) => RouteOutcome::Routed {
            class,
            queue: q.to_string(),
        },
        Some(_) => RouteOutcome::Escalated {
            queue: ESCALATION_QUEUE,
            reason: EscalateReason::UndeclaredQueue,
        },
    }
}

/// One agent's declared load headroom.
///
/// A **declared constant per agent**, supplied by the caller because the agent
/// registry does not exist yet. Keeping it a parameter rather than a constant
/// here is what makes the capacity model refutable: queue data will confirm or
/// contradict whatever envelope a caller declares, rather than the function
/// quietly owning a number nobody chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadProfile {
    pub principal: String,
    /// Cases currently open for this agent.
    pub open_cases: i32,
    /// The agent's declared headroom. Availability requires a load strictly
    /// below this, so an agent exactly at their envelope is full.
    pub load_envelope: i32,
}

/// Which of the three availability conditions an offer rests on.
///
/// Always at least one; the conjunction is in [`Offer::basis`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferBasis {
    Idle,
    Channel,
}

/// A candidate for the work, offered — never handed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub principal: String,
    pub basis: OfferBasis,
    /// The conditions this offer actually rests on, in the order they were
    /// checked. Always contains `OnShift` and `UnderEnvelope`; `Presence`
    /// carries which activity kind qualified.
    pub basis_flags: Vec<&'static str>,
}

/// The agents who may be offered the work, and why.
///
/// No assignee. No writer. Nothing here moves a case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offers {
    pub worktype: String,
    pub offered: Vec<Offer>,
}

impl Offers {
    /// True when nobody qualified. This is a normal outcome, not an error: a
    /// full room is a reason to wait, not a reason to assign someone anyway.
    pub fn is_empty(&self) -> bool {
        self.offered.is_empty()
    }
}

/// The agents eligible to be offered work of `worktype`.
///
/// `required_skills` is passed in rather than derived from the worktype: the
/// worktype→skills resolution is knowledge-ring content and is proposal-gated,
/// so it is not invented here. A caller with no proposal supplies an empty
/// slice, which matches every agent rather than guessing a requirement.
///
/// Availability is the conjunction the capacity model names: an active shift
/// window for this domain, a presence activity kind that means free, and an
/// open-case load strictly below the agent's declared envelope. All three, or
/// the agent is not offered.
pub fn select_assignee(
    worktype: &str,
    domain: &str,
    now: i64,
    presence: &[CrewMember],
    shifts: &[Shift],
    load: &[LoadProfile],
    required_skills: &[&str],
) -> Offers {
    let roster: &[String] = active_shift(shifts, domain, now)
        .map(|s| s.roster.as_slice())
        .unwrap_or(&[]);

    let mut offered = Vec::new();
    for member in presence {
        if !roster.iter().any(|p| p == &member.principal) {
            continue;
        }
        let Some(basis) = AVAILABLE_KINDS.iter().find(|k| **k == member.activity_kind) else {
            continue;
        };
        // A member whose activity kind is not in the closed vocabulary at all
        // is treated as unavailable rather than as free.
        if !ACTIVITY_KINDS.contains(&member.activity_kind.as_str()) {
            continue;
        }
        if !required_skills
            .iter()
            .all(|s| member.skills.iter().any(|m| m == s))
        {
            continue;
        }
        let Some(profile) = load.iter().find(|p| p.principal == member.principal) else {
            continue;
        };
        if profile.open_cases >= profile.load_envelope {
            continue;
        }
        offered.push(Offer {
            principal: member.principal.clone(),
            basis: if *basis == "idle" {
                OfferBasis::Idle
            } else {
                OfferBasis::Channel
            },
            basis_flags: vec!["Presence", "OnShift", "UnderEnvelope"],
        });
    }

    Offers {
        worktype: worktype.to_string(),
        offered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::crew::PresenceState;

    const DECLARED: [&str; 3] = ["Q-VSAN", "Q-OS", "Q-HW-STORAGE"];

    fn member(principal: &str, kind: &str, skills: &[&str]) -> CrewMember {
        CrewMember {
            principal: principal.to_string(),
            state: PresenceState::Active,
            activity_kind: kind.to_string(),
            current_case_ref: None,
            site: None,
            roles: vec![],
            skills: skills.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    fn shift(roster: &[&str]) -> Shift {
        Shift {
            id: 1,
            domain: "global".to_string(),
            site: "default".to_string(),
            tz: "UTC".to_string(),
            start_epoch: 1_000,
            end_epoch: 2_000,
            overlap_minutes: 0,
            roster: roster.iter().map(|r| (*r).to_string()).collect(),
        }
    }

    fn load(principal: &str, open: i32, envelope: i32) -> LoadProfile {
        LoadProfile {
            principal: principal.to_string(),
            open_cases: open,
            load_envelope: envelope,
        }
    }

    // ── the routing axis cannot drift from the classifier ────────────────

    #[test]
    fn the_routing_axis_is_the_classifier_vocabulary_exactly() {
        let ours: Vec<&str> = RoutingClass::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            ours,
            crate::procedural::CATEGORIES.to_vec(),
            "the routing axis has drifted from the classifier that feeds it. A class nothing \
             emits routes into a vocabulary nothing produces; a class we invented routes nowhere."
        );
    }

    #[test]
    fn every_classifier_category_resolves_to_its_own_class() {
        // The resolver's vocabulary is the one the classifier emits, so this walks
        // the classifier's list rather than a restated one: a category added to
        // CATEGORIES that the resolver cannot reach fails here. That is also what
        // catches a hand-typed label list going stale — a resolver that forgot a
        // category cannot pass this loop, which is the drift the search exists to
        // make impossible and the honest test of.
        for label in crate::procedural::CATEGORIES {
            let class = RoutingClass::from_label(label)
                .unwrap_or_else(|| panic!("{label} is a classifier category with no class"));
            assert_eq!(
                class.as_str(),
                *label,
                "{label} resolved to a class that reports a different name — the label vocabulary \
                 and the class vocabulary are two lists, and one of them is wrong"
            );
        }
    }

    #[test]
    fn the_resolver_answers_no_class_rather_than_guessing_one() {
        // Each of these is a label this build does not recognise. None may become
        // a class, and least of all the catch-all: a case routed under a class the
        // classifier never emitted is routed on nothing.
        for label in [
            "",
            "human_unmeasured",
            "technolog",  // truncated
            "Technology", // not case folded
            "business process",
            "general ", // trailing whitespace
            "Q-OPS-ESCALATION",
        ] {
            assert_eq!(
                RoutingClass::from_label(label),
                None,
                "{label:?} resolved to a class; an unrecognised label must be no class"
            );
        }
    }

    #[test]
    fn every_class_resolves_back_to_itself() {
        // The round trip, both directions. A class whose own name does not resolve
        // to it could still satisfy the two pins above — this is what makes the
        // resolver total over ALL rather than accidentally correct on the labels
        // the classifier happens to emit today.
        for class in RoutingClass::ALL {
            assert_eq!(
                RoutingClass::from_label(class.as_str()),
                Some(class),
                "{class:?} does not resolve from its own name; a class nothing can name is a \
                 class no caller can reach"
            );
        }
        assert_eq!(
            RoutingClass::ALL.len(),
            crate::procedural::CATEGORIES.len(),
            "the resolver searches ALL, so the two vocabularies must be the same size as well as \
             the same contents"
        );
    }

    // ── unknown escalates; it never becomes the nearest match ─────────────

    #[test]
    fn an_undeclared_queue_escalates_and_never_becomes_its_nearest_match() {
        // Each of these is *close* to a declared queue, and a nearest-match
        // implementation would resolve several of them. All must escalate.
        let near_misses = [
            "Q-VSANN",    // one character off
            "Q-VSAN ",    // trailing whitespace
            " q-vsan",    // case folded
            "Q-VSA",      // prefix
            "Q-VSAN-NEW", // superset
        ];
        for q in near_misses {
            let out = route_to_queue(RoutingClass::Technology, Some(q), &DECLARED, Some(9_999));
            assert_eq!(
                out,
                RouteOutcome::Escalated {
                    queue: ESCALATION_QUEUE,
                    reason: EscalateReason::UndeclaredQueue,
                },
                "{q:?} is not a declared queue. Inventing a destination nobody declared is worse \
                 than admitting the case needs a human."
            );
        }
    }

    #[test]
    fn a_resolved_declared_queue_routes_and_carries_the_class() {
        let out = route_to_queue(
            RoutingClass::Infrastructure,
            Some("Q-VSAN"),
            &DECLARED,
            None,
        );
        assert_eq!(
            out,
            RouteOutcome::Routed {
                class: RoutingClass::Infrastructure,
                queue: "Q-VSAN".to_string(),
            }
        );
        assert!(!out.is_escalated());
        assert_eq!(out.queue(), "Q-VSAN");
    }

    #[test]
    fn no_candidate_escalates() {
        assert_eq!(
            route_to_queue(RoutingClass::General, None, &DECLARED, None),
            RouteOutcome::Escalated {
                queue: ESCALATION_QUEUE,
                reason: EscalateReason::NoCandidate,
            }
        );
    }

    // ── no confidence may bypass the escalation queue ─────────────────────

    #[test]
    fn no_confidence_value_bypasses_escalation() {
        // The whole plausible range, plus the extremes. If any of these
        // produced a Routed outcome for an undeclared queue, a threshold had
        // been invented somewhere between the parameter and the match.
        for confidence in [-1, 0, 1, 5_000, 7_000, 9_999, 10_000, i32::MAX] {
            let out = route_to_queue(
                RoutingClass::Technology,
                Some("Q-NOT-A-QUEUE"),
                &DECLARED,
                Some(confidence),
            );
            assert!(
                out.is_escalated(),
                "confidence {confidence} routed an undeclared queue to {out:?}. Confidence is a \
                 quality signal; whether a destination exists is a fact about the vocabulary."
            );
        }
        // And the inverse: a low confidence does not *demote* a declared queue.
        for confidence in [-1, 0, 5_000, i32::MAX] {
            assert!(
                !route_to_queue(
                    RoutingClass::Technology,
                    Some("Q-OS"),
                    &DECLARED,
                    Some(confidence)
                )
                .is_escalated(),
                "confidence {confidence} escalated a declared queue. The number is read and \
                 discarded; it decides nothing in either direction."
            );
        }
    }

    #[test]
    fn confidence_is_discarded_and_cannot_be_read_by_any_branch() {
        // The control is a binding, so a pin asserting "it is not read" is only
        // worth anything if reading it anywhere would break the build. This
        // asserts the outcome is identical across the range, which is the
        // observable form of the same claim.
        let outcomes: Vec<RouteOutcome> = (-10..=10)
            .map(|d| {
                route_to_queue(
                    RoutingClass::Vendor,
                    Some("Q-OS"),
                    &DECLARED,
                    Some(d * 1_000),
                )
            })
            .collect();
        assert!(
            outcomes.windows(2).all(|w| w[0] == w[1]),
            "the outcome varied with confidence: {outcomes:?}"
        );
    }

    // ── offers, structurally: there is no assignee to hold ───────────────

    #[test]
    fn offers_cannot_express_an_assignment() {
        // Exhaustive destructuring is the pin: it compiles only while these
        // are exactly the fields. Adding an `assignee` to either type breaks
        // this test at compile time, which is the strongest form available.
        let offers = select_assignee("storage", "global", 1_500, &[], &[], &[], &[]);
        let Offers { worktype, offered } = offers;
        assert_eq!(worktype, "storage");
        assert!(offered.is_empty());

        let offer = Offer {
            principal: "p".to_string(),
            basis: OfferBasis::Idle,
            basis_flags: vec![],
        };
        let Offer {
            principal,
            basis,
            basis_flags,
        } = offer;
        assert_eq!(principal, "p");
        assert_eq!(basis, OfferBasis::Idle);
        assert!(basis_flags.is_empty());
    }

    // ── the availability conjunction ──────────────────────────────────────

    #[test]
    fn availability_requires_all_three_conditions() {
        let presence = vec![
            member("free", "idle", &[]),
            member("oncall", "channel", &[]),
            member("busy", "cranking", &[]),
            member("reviewing", "reviewing", &[]),
            member("offshift", "idle", &[]),
            member("full", "idle", &[]),
            member("unknown-kind", "lunch", &[]),
            member("no-profile", "idle", &[]),
        ];
        let shifts = vec![shift(&[
            "free",
            "oncall",
            "busy",
            "reviewing",
            "full",
            "unknown-kind",
            "no-profile",
        ])];
        let load = vec![
            load("free", 0, 3),
            load("oncall", 2, 3),
            load("busy", 0, 3),
            load("reviewing", 0, 3),
            load("full", 3, 3), // exactly at the envelope is NOT under it
            load("unknown-kind", 0, 3),
        ];

        let offers = select_assignee("storage", "global", 1_500, &presence, &shifts, &load, &[]);
        let names: Vec<&str> = offers
            .offered
            .iter()
            .map(|o| o.principal.as_str())
            .collect();
        assert_eq!(
            names,
            ["free", "oncall"],
            "expected exactly the idle-on-shift-under-envelope agent and the on-call equivalent."
        );
    }

    /// Trap: a fixture that claims to exclude one condition must be shown to
    /// exclude *that* condition and no other. Each row differs from the
    /// accepting fixture in exactly one fact.
    #[test]
    fn each_condition_excludes_independently_and_alone() {
        let base = (
            vec![member("a", "idle", &[])],
            vec![shift(&["a"])],
            vec![load("a", 0, 3)],
            1_500,
        );

        let offered = |p: Vec<CrewMember>, s: Vec<Shift>, l: Vec<LoadProfile>, now: i64| {
            select_assignee("w", "global", now, &p, &s, &l, &[]).is_empty()
        };

        // The baseline really is offered — otherwise every "excluded" below
        // would pass for the wrong reason.
        assert!(
            !offered(base.0.clone(), base.1.clone(), base.2.clone(), base.3),
            "the baseline fixture must be offered; otherwise the exclusions prove nothing"
        );

        // Exactly one fact changed in each case.
        assert!(
            offered(
                base.0.clone(),
                base.1.clone(),
                vec![load("a", 3, 3)],
                base.3
            ),
            "load at the envelope must exclude"
        );
        assert!(
            offered(
                base.0.clone(),
                base.1.clone(),
                vec![load("a", 9, 3)],
                base.3
            ),
            "load over the envelope must exclude"
        );
        assert!(
            offered(
                base.0.clone(),
                vec![shift(&["other"])],
                base.2.clone(),
                base.3
            ),
            "being off the active shift's roster must exclude"
        );
        assert!(
            offered(base.0.clone(), base.1.clone(), base.2.clone(), 9_999),
            "a time outside the shift window must exclude"
        );
        assert!(
            offered(
                vec![member("a", "cranking", &[])],
                base.1.clone(),
                base.2.clone(),
                base.3
            ),
            "an activity kind outside the free pair must exclude"
        );
    }

    #[test]
    fn a_required_skill_the_agent_lacks_excludes_them() {
        let presence = vec![member("a", "idle", &["storage"]), member("b", "idle", &[])];
        let shifts = vec![shift(&["a", "b"])];
        let load = vec![load("a", 0, 3), load("b", 0, 3)];

        let offers = select_assignee(
            "w",
            "global",
            1_500,
            &presence,
            &shifts,
            &load,
            &["storage"],
        );
        let names: Vec<&str> = offers
            .offered
            .iter()
            .map(|o| o.principal.as_str())
            .collect();
        assert_eq!(names, ["a"]);
    }

    #[test]
    fn an_empty_room_is_a_normal_outcome_not_an_assignment() {
        let offers = select_assignee(
            "w",
            "global",
            1_500,
            &[member("a", "idle", &[])],
            &[shift(&["a"])],
            &[load("a", 5, 3)],
            &[],
        );
        assert!(offers.is_empty());
        assert_eq!(offers.worktype, "w");
    }
}
