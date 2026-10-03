//! The routing seam: a case and a roster, routed through the core.
//!
//! ## What this wires, and what it deliberately does not
//!
//! [`crate::workflow::routing`] shipped as a pure core with **zero production
//! callers** — a decision nothing could reach. This module is its first consumer,
//! and it exists to make that false.
//!
//! **It holds no class→queue table, and this is the control.** The queue vocabulary
//! belongs to the taxonomy that declares it, and `routing.rs` states the law: a
//! second copy in this crate would be exactly the hand-typed list that had to be
//! removed from the generator upstream — a copy that can fall behind its source
//! with nothing failing. So the vocabulary is **read from the declared proposal
//! surface**, never typed here, and a case whose queue is not declared escalates.
//!
//! Inventing a domain→queue map to make routing *look* productive is the move
//! three earlier increments each refused, and it is the one this seam exists to
//! make unnecessary: routing works today with whatever the taxonomy has actually
//! declared, and escalates on everything else.
//!
//! ## Offers, never assignments
//!
//! [`offers_for`] returns [`crate::workflow::routing::Offer`]s. That type has no
//! assignee field and no writer behind it, so this module cannot assign work even
//! by accident — see `routing_offers_never_assign` in the pin file, which asserts
//! the **type** rather than scanning source.

use rusqlite::Connection;

use crate::workflow::crew::{self, CrewMember};
use crate::workflow::routing::{
    self, ESCALATION_QUEUE, LoadProfile, Offer, Offers, RouteOutcome, RoutingClass,
};

/// The routing seam's typed refusal.
///
/// Every failure is a named cause. There is no variant meaning "try again with a
/// different queue", because a caller that could retry with a different queue would
/// be choosing the destination the routing core refused to invent.
///
/// The upstream causes are carried as **rendered strings**, deliberately: the
/// frozen `CrewError` and `ShiftError` enums are not `Clone + Eq`, and widening
/// either is a change to a published type this seam has no standing to make. The
/// named [`RoutingError`] variant is the contract; the cause inside it is
/// diagnostic. This is the same separation the replay gate drew between its
/// refusal code and its refusal reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingError {
    /// The domain was empty or whitespace.
    UnknownDomain { domain: String },
    /// The crew roster could not be read.
    Crew { domain: String, reason: String },
    /// The declared shift windows could not be read.
    Shifts { domain: String, reason: String },
    /// The declared queue vocabulary could not be read.
    Queues { domain: String, reason: String },
}

impl std::fmt::Display for RoutingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownDomain { domain } => write!(f, "unknown domain: {domain}"),
            Self::Crew { domain, reason } => write!(f, "crew read failed for {domain}: {reason}"),
            Self::Shifts { domain, reason } => {
                write!(f, "shift read failed for {domain}: {reason}")
            }
            Self::Queues { domain, reason } => {
                write!(f, "queue vocabulary read failed for {domain}: {reason}")
            }
        }
    }
}

/// Route one case, reading the declared vocabulary from the proposal surface.
///
/// `proposed` is whatever the upstream classifier or proposal produced. It is
/// **not** trusted: a queue that the declared vocabulary does not contain escalates
/// rather than resolving to the nearest known queue.
///
/// Returns the outcome plus the vocabulary it was routed against, so a receipt can
/// say which declaration decided the case rather than only where it went.
pub fn route_case(
    conn: &Connection,
    domain: &str,
    class: RoutingClass,
    proposed: Option<&str>,
    confidence: Option<i32>,
) -> Result<RoutedCase, RoutingError> {
    if domain.trim().is_empty() {
        return Err(RoutingError::UnknownDomain {
            domain: domain.to_string(),
        });
    }

    let declared = declared_queues(conn, domain).map_err(|reason| RoutingError::Queues {
        domain: domain.to_string(),
        reason,
    })?;

    // Borrowed from the declared vocabulary, never owned here: a `String` the
    // caller passed would not outlive the call, and an owned copy of the
    // escalation id would be a second declaration of it.
    let borrowed: Vec<&str> = declared.iter().map(String::as_str).collect();

    let outcome = routing::route_to_queue(class, proposed, &borrowed, confidence);

    Ok(RoutedCase {
        domain: domain.to_string(),
        class,
        outcome,
        declared_queues: declared,
    })
}

/// One case's routing result, with the vocabulary that decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedCase {
    pub domain: String,
    /// The classifier's verdict, carried for the receipt.
    pub class: RoutingClass,
    pub outcome: RouteOutcome,
    /// The queue ids the taxonomy declared for this domain, sorted and de-duped.
    /// Empty means the domain has declared nothing, and every case escalates.
    pub declared_queues: Vec<String>,
}

impl RoutedCase {
    /// The queue that owns the case, whichever outcome this is.
    pub fn queue(&self) -> &str {
        self.outcome.queue()
    }

    /// Whether the case went to the human escalation queue.
    pub fn is_escalated(&self) -> bool {
        self.outcome.is_escalated()
    }
}

/// The queue ids this domain has declared.
///
/// Read from the **governed proposal surface**, so a queue exists here only once
/// something proposed it and that proposal was applied. A routing declaration
/// rides the existing `proposals` table rather than a table of its own: a new
/// table would be a second way to declare a queue, and two ways is how a
/// vocabulary drifts.
///
/// **This is the anti-invention seam.** There is no class→queue map anywhere in
/// this module or the core behind it. A queue becomes routable when the taxonomy
/// declares it, and a class without a declaration escalates to a human — which is
/// the correct answer while the class-side map is proposal-gated knowledge-ring
/// content.
const QUEUE_DECLARATION_KIND: &str = "queue_declaration";

/// The bounds law: routing resolves by exact membership, so the cap cannot change
/// a verdict for any case whose queue is declared. It bounds the receipt.
const MAX_DECLARED_QUEUES: usize = 256;

fn declared_queues(conn: &Connection, domain: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT json_extract(payload_json, '$.queue')
             FROM proposals
             WHERE domain = ?1 AND kind = ?2 AND status = 'approved'
               AND json_extract(payload_json, '$.queue') IS NOT NULL
             ORDER BY 1 LIMIT ?3",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params![domain, QUEUE_DECLARATION_KIND, MAX_DECLARED_QUEUES as i64],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Offer the roster members who could take this worktype's case.
///
/// **Offers, never assignments.** The returned [`Offers`] carries candidates and
/// nothing else; there is no function in this module that writes an assignment, so
/// "the machine decided who works" is not expressible here rather than merely
/// discouraged.
#[allow(clippy::too_many_arguments)]
pub fn offers_for(
    conn: &Connection,
    domain: &str,
    worktype: &str,
    required_skills: &[&str],
    load: &[LoadProfile],
    now: i64,
) -> Result<Offers, RoutingError> {
    if domain.trim().is_empty() {
        return Err(RoutingError::UnknownDomain {
            domain: domain.to_string(),
        });
    }
    let presence: Vec<CrewMember> =
        crew::roster(conn, domain, now).map_err(|reason| RoutingError::Crew {
            domain: domain.to_string(),
            reason: reason.to_string(),
        })?;
    // The shift window is read from the shipped `shifts` table through its own
    // reader, so this seam adds no table and no second shift store.
    let shifts = crate::workflow::shifts::list_shifts(conn, domain).map_err(|reason| {
        RoutingError::Shifts {
            domain: domain.to_string(),
            reason: reason.to_string(),
        }
    })?;
    let offers = routing::select_assignee(
        worktype,
        domain,
        now,
        &presence,
        &shifts,
        load,
        required_skills,
    );
    Ok(offers)
}

/// Re-exported so a receipt can name the escalation destination without
/// restating the id.
pub const HUMAN_ESCALATION_QUEUE: &str = ESCALATION_QUEUE;

/// Re-exported so a caller building offers needs no second import.
pub type OfferList = Vec<Offer>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::routing::OfferBasis;

    fn conn() -> Connection {
        let c = rusqlite::Connection::open_in_memory().expect("in-memory");
        c.execute_batch(
            "CREATE TABLE proposals(
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 domain TEXT NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL,
                 payload_json TEXT NOT NULL DEFAULT '{}');
             CREATE TABLE shifts(
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 domain TEXT NOT NULL, site TEXT NOT NULL,
                 tz TEXT NOT NULL DEFAULT 'UTC',
                 start_epoch INTEGER NOT NULL, end_epoch INTEGER NOT NULL,
                 overlap_minutes INTEGER NOT NULL DEFAULT 0,
                 roster_json TEXT NOT NULL DEFAULT '[]',
                 created_at INTEGER NOT NULL);",
        )
        .expect("schema");
        c
    }

    /// Declare a queue through the governed proposal surface.
    fn declare(c: &Connection, domain: &str, queue: &str) {
        c.execute(
            "INSERT INTO proposals(domain, kind, status, payload_json)
             VALUES (?1, 'queue_declaration', 'approved', json_object('queue', ?2))",
            rusqlite::params![domain, queue],
        )
        .expect("seed declaration");
    }

    #[test]
    fn an_undeclared_queue_escalates_and_never_resolves_to_a_neighbour() {
        let c = conn();
        declare(&c, "care", "Q-CARE-INTAKE");
        // 'Q-CARE-INTAKE-2' shares a prefix with the declared queue. A prefix or
        // edit-distance match would route it; the core refuses.
        let routed = route_case(
            &c,
            "care",
            RoutingClass::BusinessProcess,
            Some("Q-CARE-INTAKE-2"),
            None,
        )
        .expect("routed");
        assert!(routed.is_escalated(), "an undeclared queue must escalate");
        assert_eq!(routed.queue(), HUMAN_ESCALATION_QUEUE);
    }

    #[test]
    fn a_declared_queue_routes_and_the_receipt_names_the_vocabulary() {
        let c = conn();
        declare(&c, "care", "Q-CARE-INTAKE");
        let routed = route_case(
            &c,
            "care",
            RoutingClass::BusinessProcess,
            Some("Q-CARE-INTAKE"),
            Some(9999),
        )
        .expect("routed");
        assert!(!routed.is_escalated());
        assert_eq!(routed.queue(), "Q-CARE-INTAKE");
        assert_eq!(routed.declared_queues, vec!["Q-CARE-INTAKE".to_string()]);
    }

    #[test]
    fn an_unapproved_declaration_is_not_a_queue() {
        let c = conn();
        // The governance surface: a proposal nobody approved declares nothing.
        c.execute(
            "INSERT INTO proposals(domain, kind, status, payload_json)
             VALUES ('care', 'queue_declaration', 'pending', json_object('queue', 'Q-CARE-INTAKE'))",
            [],
        )
        .expect("seed");
        let routed = route_case(
            &c,
            "care",
            RoutingClass::BusinessProcess,
            Some("Q-CARE-INTAKE"),
            None,
        )
        .expect("routed");
        assert!(
            routed.is_escalated(),
            "a pending declaration must not route"
        );
        assert!(routed.declared_queues.is_empty());
    }

    #[test]
    fn a_domain_that_declares_nothing_escalates_everything() {
        let c = conn();
        let routed = route_case(&c, "care", RoutingClass::General, Some("Q-ANYTHING"), None)
            .expect("routed");
        assert!(routed.is_escalated());
        assert!(routed.declared_queues.is_empty());
    }

    #[test]
    fn an_empty_domain_refuses_rather_than_routing() {
        let c = conn();
        assert!(matches!(
            route_case(&c, "  ", RoutingClass::General, None, None),
            Err(RoutingError::UnknownDomain { .. })
        ));
    }

    #[test]
    fn offers_are_empty_when_no_shift_is_declared() {
        let c = conn();
        let offers = offers_for(&c, "care", "w-intake", &[], &[], 1_000).expect("offers");
        assert!(
            offers.offered.is_empty(),
            "no declared shift means nobody is on shift, so no offer may be made"
        );
    }

    #[test]
    fn a_malformed_roster_declaration_offers_nobody() {
        let c = conn();
        // The shipped shift reader decodes the roster; malformed content must not
        // become availability.
        c.execute(
            "INSERT INTO shifts(domain, site, start_epoch, end_epoch, roster_json, created_at)
             VALUES ('care', 'main', 0, 9999, 'not json', 0)",
            [],
        )
        .expect("seed");
        let offers = offers_for(&c, "care", "w-intake", &[], &[], 1_000);
        // Either the reader refuses or it yields nobody; both are fail-closed, and
        // what must not happen is an offer being made off unparseable content.
        if let Ok(offers) = offers {
            assert!(
                offers.offered.is_empty(),
                "unparseable governance content must not become availability"
            );
        }
    }

    #[test]
    fn the_escalation_queue_is_named_once() {
        // The id is re-exported, never re-typed: a second literal would be a
        // second declaration that could drift from the core's.
        assert_eq!(HUMAN_ESCALATION_QUEUE, routing::ESCALATION_QUEUE);
    }

    #[test]
    fn an_offer_carries_a_basis_and_no_assignment() {
        // The Offer type has no assignee field; this test names the fields that
        // exist so a future one has to be added here to compile.
        let offer = Offer {
            principal: "agent-a".to_string(),
            basis: OfferBasis::Idle,
            basis_flags: vec!["Presence"],
        };
        let Offer {
            principal,
            basis,
            basis_flags,
        } = offer;
        assert_eq!(principal, "agent-a");
        assert_eq!(basis, OfferBasis::Idle);
        assert_eq!(basis_flags.len(), 1);
    }
}
