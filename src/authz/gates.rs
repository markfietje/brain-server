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
/// **The `publish` finding, stated exactly.** `CAN_ACTIONS`
/// (`src/role.rs`) does not name `publish`. `Role::validate` returns `Err` for
/// any `can` item outside it. The only production writer of the `roles` table
/// is `src/handlers/roles.rs`'s upsert, which calls `validate()`, and the
/// migration seeds thirteen fixed presets, none of which carries `publish`.
/// No production path can therefore store a role holding `publish`, so
/// `authorize_role(.., "publish")` denies every principal that HAS roles —
/// including the `admin` preset — and passes every principal that has none.
///
/// **What that means, stated plainly rather than softened:** KCS article
/// publication is impossible for every role-bearing principal today. That is a
/// real defect, it predates this round , and this round does not fix it: the fix is minting
/// `publish` into `CAN_ACTIONS`, and the frozen-vocabulary decision freezes the vocabulary for this round.
/// It is filed, not silently absorbed, and `r47_publish_is_a_deny_only_handler_
/// deliberately, and `r47_publish_is_a_deny_only_handler_seam_capability` pins the
/// premise so the class cannot outlive it quietly.
///
/// **The false precedent, recorded because the round nearly inherited it.** The
/// obvious argument for pinning this as intended is that `workflow` is the same
/// class. It is not: `CAN_ACTIONS` DOES name `workflow`, and the shipped
/// `workflow-operator` preset grants exactly `can:["workflow"]`. Two in-tree
/// comments claimed otherwise and were wrong.
pub const DENY_ONLY_CAPABILITIES: &[&str] = &["publish"];

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

    #[test]
    fn r47_the_deny_only_class_is_frozen_and_named() {
        assert_eq!(DENY_ONLY_CAPABILITIES, &["publish"]);
        assert!(is_deny_only("publish"));
        assert!(!is_deny_only("approve"));
        assert!(!is_deny_only("workflow"));
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
    #[test]
    fn r47_gate_rows_read_their_declared_action() {
        let stats = gate_for("/stats").expect("/stats is gated");
        assert_eq!(stats.required_action, Action::Read);
        assert!(!stats.public);

        let audit = gate_for("/audit").expect("/audit is gated");
        assert_eq!(audit.required_action, Action::Admin);
        assert!(!audit.public);
    }
}
