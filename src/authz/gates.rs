//! The gate declaration, and the deny-only capability class.
//!
//! **One declaration, two consumers .** The route→action table lives in
//! `server::router::route_guards` and has ALWAYS been production data — the
//! module is declared `pub` at `server/router/mod.rs` with no `cfg(test)`
//! gate, and both auth middlewares already call `is_public_path` through it.
//! The execution prompt's §2.3 was right that a stale doc comment in that
//! file claimed the opposite; it was wrong that this module had to be
//! "promoted" first, because the promotion already happened. This module is the
//! seam that lets the RUNTIME read the same rows the coverage pin scans, so
//! "I updated the table but not the middleware" stops being expressible.
//!
//! **The capability axis, and why it is nearly empty here.** The plan's
//! `Gate` carries a per-route `required_capability`. Populating it faithfully
//! is not possible and pretending otherwise would be a second KILL 3. The
//! reason is measured: `src/handlers/gate.rs` calls
//! `authorize_role(&principal.0, &pool, "publish")` *conditionally on a request
//! body field* (`kind == kcs_publish`), inside `/proposals/{id}/approve` — a
//! route that also requires `approve` and has a `remedy` branch. A middleware
//! keyed on `(MatchedPath, Method)` cannot see a body. Declaring
//! `required_capability: "publish"` for that path would deny EVERY approval on
//! it, not just the publish ones, which is an unreviewed authorization change.
//!
//! So the capability stays where it can see the body — the handler's own
//! `authorize_role` (the defence-in-depth rule's inner gate) — and this module names the one class of
//! capability that is a permanent deny by construction, so the refusal is
//! declared rather than implied.

use crate::auth::policy::Action;
use crate::server::router::route_guards::AUTHZ_GATES;

/// Capabilities that exist OUTSIDE `CAN_ACTIONS` and are therefore
/// unsatisfiable by construction.
///
/// **THE CLASS IS NOW EMPTY, and that is the finding, not a leftover.** `publish`
/// was the only member: `CAN_ACTIONS` did not name it, `Role::validate` rejects
/// any `can` item outside it, and the only production writer of the roles table
/// validates — so no role could hold `publish`, and `authorize_role(..,
/// "publish")` denied every role-bearing principal (including `admin`) while
/// passing every role-less one. KCS article publication was therefore impossible
/// for anyone the RBAC layer could see, which is a defect, not a design.
///
/// A capability can only be DENY-ONLY by construction if the vocabulary cannot
/// name it. That is now false: `publish` is in `CAN_ACTIONS`, granted
/// deliberately to the four roles that already carry `approve` (publication
/// stays its OWN capability — approval does not imply it), and refused for
/// every other role.
///
/// The list and `is_deny_only` stay, because the machinery they feed is real
/// and the class is how a future vocabulary change is declared rather than
/// inferred. An empty list means "nothing is deny-only right now", which is a
/// fact worth having in one place: a reader who assumed the class still held
/// `publish` would be reasoning from a stale premise, and the pin in
/// `tests/rbac_evaluation_pins.rs` is what catches that.
///
/// **The false precedent, recorded because the original round nearly inherited
/// it.** The obvious argument for pinning `publish` as deny-only was that
/// `workflow` is the same class. It is not: `CAN_ACTIONS` names `workflow` and
/// the shipped `workflow-operator` preset grants exactly `can:["workflow"]`.
///
/// **ponytail:** no new capability beyond the one the handlers already asked
/// for; no change to the approval path — publication remains a separate verb
/// with its own gate.
pub const DENY_ONLY_CAPABILITIES: &[&str] = &[];

/// Whether `capability` is in the frozen deny-only class.
pub fn is_deny_only(capability: &str) -> bool {
    DENY_ONLY_CAPABILITIES.contains(&capability)
}

/// A gate row read out of the shared table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    Read,
    Write,
    Admin,
    /// The `public` marker: the middleware exempts the path and the handler
    /// carries no `authorize()`.
    Public,
}

impl RowAction {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "Read" => Some(RowAction::Read),
            "Write" => Some(RowAction::Write),
            "Admin" => Some(RowAction::Admin),
            "public" => Some(RowAction::Public),
            _ => None,
        }
    }
}

/// Routes that are NEITHER public NOR gated in the shared table, and why each
/// is not a hole.
///
/// **This is the round's first measured false positive, and it is recorded
/// because the fix is the interesting part.** The first cut of the middleware
/// denied any non-public route with no table row. That is correct in principle
/// and WRONG in fact: seven registered non-public routes carry no row by
/// DESIGN. Three are presentation-gated (the verified bearer IS the gate), and
/// four are registered only under the `compliance-pack` feature, so a table row
/// would be vacuous in a default build. The pin
/// `health_db_admin_full_read_reduced` in `tests/authz_matrix.rs` moved from 200
/// to 403 the moment the middleware landed — KILL 1, caught by running the
/// matrix rather than by reasoning about it.
///
/// The exemption used to live as a test-local `ALLOWLIST` const inside
/// `tests/main_suite.rs`, which meant production could not see it and the
/// middleware had no choice but to call those routes ungated. It is declared
/// HERE now, beside the table it qualifies, and the test reads this list — the
/// same one-declaration/two-consumers rule the gate table itself follows.
pub const PRESENTATION_GATED: &[(&str, &str)] = &[
    // Presentation-gated only: the middleware's verified bearer IS the gate
    // (the Drawbridge carve-out). `/health/db` is the documented carve-out;
    // `/auth/logout` revokes the presented token, so a public logout could
    // revoke nothing, and it 401s without a principal on its own.
    (
        "/health/db",
        "middleware-presentation-gated (the Drawbridge carve-out)",
    ),
    (
        "/auth/logout",
        "middleware-presentation-gated (logout revokes the bearer)",
    ),
    // Feature-gated: registered only under `compliance-pack`, so a table row
    // would be vacuous in a default build. The handlers carry their own gates.
    ("/audit/export", "feature-gated: compliance-pack"),
    (
        "/compliance/evaluation-record",
        "feature-gated: compliance-pack",
    ),
    ("/compliance/inventory", "feature-gated: compliance-pack"),
    ("/ropa", "feature-gated: compliance-pack"),
    ("/ropa/{id}", "feature-gated: compliance-pack"),
];

/// Whether `route` is a declared exemption rather than a missing gate.
pub fn is_presentation_gated(route: &str) -> bool {
    PRESENTATION_GATED.iter().any(|(p, _)| *p == route)
}

/// The gate row for a matched route PATTERN, or `None` when the table has no
/// row for it — which the caller turns into `Deny(RouteUngated)` rather than
/// letting through.
///
/// Keyed on the route PATTERN alone, because that is how the shared table is
/// keyed. The request's method is not a parameter: there is nothing to look it
/// up against, and accepting one it cannot affect would be a lie in the
/// signature. The method still reaches `policy::decide`, which is where a
/// method-keyed row would be enforced.
pub fn gate_for(route: &str) -> Option<crate::authz::policy::Gate> {
    // The table is keyed by path, not (path, method): several paths carry
    // several methods under one action, and two paths carry two DIFFERENT
    // actions. Taking the FIRST row for the path matches what the existing
    // authz matrix already pairs, so the runtime and the receipt agree.
    let (path, raw_action) = AUTHZ_GATES.iter().find(|(p, _)| *p == route).copied()?;
    let action = RowAction::parse(raw_action)?;
    let (public, required_action) = match action {
        RowAction::Public => (true, Action::Read),
        RowAction::Read => (false, Action::Read),
        RowAction::Write => (false, Action::Write),
        RowAction::Admin => (false, Action::Admin),
    };
    Some(crate::authz::policy::Gate {
        route: path,
        // The shared table carries no method column, so the row is not
        // method-keyed; method-level policy stays the handler's per-method
        // `authorize`, which is per-method by construction.
        method_policy: crate::authz::policy::MethodPolicy::Any,
        required_action,
        // Never populated from the table: see the module doc on why the
        // capability axis is not expressible per (path, method).
        required_capability: "",
        public,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::policy::Action;
    use crate::authz::policy::{DenyReason, Verdict, decide_gate_verdict};

    /// The class that once held `publish` is EMPTY, because `publish` is now
    /// in `CAN_ACTIONS` and grantable. This pin is the check on THAT claim:
    /// a reader who found `publish` back in the deny-only list — or a future
    /// edit that put it there to make publication impossible again — fails
    /// here, loudly, with the reason written down.
    #[test]
    fn the_deny_only_class_is_empty_because_publish_is_now_grantable() {
        assert!(
            DENY_ONLY_CAPABILITIES.is_empty(),
            "no capability is deny-only by construction any more; if you are adding one, say \
             why the vocabulary cannot name it"
        );
        assert!(!is_deny_only("publish"));
        assert!(!is_deny_only("approve"));
        assert!(!is_deny_only("workflow"));
        // The premise, asserted: `publish` really is nameable now.
        assert!(
            crate::role::CAN_ACTIONS.contains(&"publish"),
            "`publish` must be in CAN_ACTIONS — the class is empty BECAUSE the vocabulary \
             can now name it, and a reader checking one without the other is reading a \
             stale premise"
        );
    }

    #[test]
    fn r47_an_unknown_route_has_no_gate() {
        assert!(
            gate_for("/definitely-not-a-route").is_none(),
            "an unmatched route must have NO gate, so the caller denies it as \
             route_ungated instead of defaulting to a permissive row"
        );
    }

    #[test]
    fn r47_a_public_row_marks_the_row_public() {
        let g = gate_for("/.well-known/security.txt").expect("security.txt is a row");
        assert!(
            g.public,
            "the security.txt row is the declared public marker"
        );
    }

    /// `/stats` is a `Read` row and `/audit` is `Admin` — both measured from
    /// the table, because a pin that assumed the wrong one would have passed on
    /// a table it never looked at.
    ///
    /// R68 "Silence" (F8-02) — **the oracle is now INDEPENDENT.** This pin
    /// previously used `gate_for` as its own oracle: it read `required_action`
    /// back out of the constructor that had just written it, so it proved the
    /// action column survives the PARSE and could not fail if enforcement was
    /// never wired. Deleting every enforcement use of `required_action` left
    /// it green — the exact drift F8-02 names.
    ///
    /// It now reads the expectation from [`AUTHZ_GATES`] — the table literal,
    /// which is the source of truth — and cross-checks the parse against it.
    /// `tests/main_suite.rs::authz_gates_cover_every_non_public_route` already
    /// pins table-to-route coverage, so this stays a parse pin rather than
    /// duplicating that one; what is NEW is that the expected value is read
    /// from the table rather than from the thing under test.
    #[test]
    fn r47_gate_rows_read_their_declared_action() {
        let expect = |route: &str| -> Action {
            let raw = AUTHZ_GATES
                .iter()
                .find(|(p, _)| *p == route)
                .map(|(_, a)| *a)
                .unwrap_or_else(|| panic!("{route} must be a row in AUTHZ_GATES"));
            match RowAction::parse(raw).expect("every row parses") {
                RowAction::Public => Action::Read,
                RowAction::Read => Action::Read,
                RowAction::Write => Action::Write,
                RowAction::Admin => Action::Admin,
            }
        };

        for route in ["/stats", "/audit"] {
            let gate = gate_for(route).unwrap_or_else(|| panic!("{route} is gated"));
            assert_eq!(
                gate.required_action,
                expect(route),
                "{route}: the parsed action must equal the TABLE's row, read independently \
                 of gate_for"
            );
            assert!(
                !gate.public,
                "{route} is a gated row, so it must not be marked public"
            );
        }
    }

    /// R68 (F8-02) — the agent-vs-Admin truth, asserted BEHAVIOURALLY against
    /// the real oracle.
    ///
    /// `src/server/router/auth.rs` used to claim the middleware "refuses the
    /// agent principal class on Admin rows by class". The oracle has no agent
    /// arm, so that claim was false while the prose survived the code change
    /// that made it false. This pin drives the production-shaped `Gate`
    /// through `decide_gate_verdict` with an `AgentLoopback` principal and
    /// asserts the verdict is **not** a `Deny`.
    ///
    /// **This pin is GREEN today and goes RED the moment someone adds the
    /// agent arm without updating the doc** — which is precisely the drift
    /// that produced F8-02. It is a doc/code agreement pin, not a policy
    /// endorsement: whether the agent class *should* be refused here is an
    /// open design question (see `policy.rs:205-220` for why the arm was
    /// deliberately removed), and this pin refuses the doc and the code
    /// disagreeing — nothing more.
    #[test]
    fn r68_the_oracle_makes_no_agent_class_refusal_and_the_doc_says_so() {
        let gate = gate_for("/audit").expect("/audit is an Admin row");
        assert_eq!(gate.required_action, Action::Admin, "measured, not assumed");
        // The REAL agent principal, built by the production constructor — not
        // a hand-rolled stand-in. A synthetic principal could differ from the
        // one the middleware actually sees, which is precisely the class of
        // self-assertion this pin exists to remove.
        let agent = crate::auth::policy::Principal::agent_loopback();
        assert_eq!(
            agent.kind,
            crate::auth::policy::PrincipalKind::AgentLoopback,
            "the constructor under test must build the class it claims"
        );
        let verdict = decide_gate_verdict(Some(&agent), &gate, "GET");
        assert!(
            !verdict.is_deny(),
            "the oracle denies an AgentLoopback principal on an Admin row. If the agent arm \
             was added on purpose, the doc at src/server/router/auth.rs must be updated in \
             the SAME commit — and this pin updated with it, because a doc claiming a \
             refusal the oracle does not make is the F8-02 defect in its original form."
        );
    }

    /// R68 (F8-02) — the two `DenyReason` arms that CANNOT fire in production.
    ///
    /// Both are unreachable because the sole production constructor hardcodes
    /// the field away (`MethodPolicy::Any` at `:163`, `required_capability: ""`
    /// at `:167`). Pinned here so the ceiling is a machine-checked FACT rather
    /// than a claim in a round note: if a future constructor populates either
    /// field, this fails and the note must be corrected in the same commit.
    #[test]
    fn r68_the_two_unreachable_deny_reasons_are_pinned_as_ceilings() {
        let gate = gate_for("/audit").expect("/audit is gated");
        assert_eq!(
            gate.method_policy,
            crate::authz::policy::MethodPolicy::Any,
            "a method-keyed row would make DenyReason::MethodNotPermitted reachable — \
             update the ceiling note in the same commit"
        );
        assert!(
            gate.required_capability.is_empty(),
            "a non-empty capability would make DenyReason::CapabilityDenyOnly reachable — \
             update the ceiling note in the same commit"
        );
        // And the arms themselves are still correct when constructed directly,
        // which is the honest scope: they are unreachable, not wrong.
        let mut method_gated = gate;
        method_gated.method_policy = crate::authz::policy::MethodPolicy::Only(&["POST"]);
        assert!(matches!(
            decide_gate_verdict(None, &method_gated, "GET"),
            Verdict::Deny(DenyReason::MethodNotPermitted)
        ));
        // The capability arm is STILL unreachable, for the same reason as the
        // method arm: `required_capability` is never populated from the table,
        // so no production Gate can carry one. It is pinned with a name that
        // is in the vocabulary now, which is the point — the arm's
        // reachability depends on the CONSTRUCTOR, not on the class being
        // non-empty.
        let mut cap_gated = gate;
        cap_gated.required_capability = "publish";
        assert!(
            !matches!(
                decide_gate_verdict(None, &cap_gated, "GET"),
                Verdict::Deny(DenyReason::CapabilityDenyOnly)
            ),
            "a gate carrying `publish` is no longer deny-only — the arm stays unreachable \
             because nothing constructs one"
        );
    }
}
