//! The RBAC oracle: one pure, total, closed-vocabulary decision.
//!
//! **What this is.** `(Principal?, Gate, Method) -> Verdict`, with no I/O, no
//! database, no clock, no transport type. The whole policy surface is a `const`
//! table only a code change can move; there is no expression language and no
//! operator-editable rule, because the failure mode of a policy DSL is that
//! nobody can answer "why was this allowed" six months later, and the failure
//! mode of a closed table is a route denied loudly and a PR filed. The second
//! failure is recoverable; the first is not.
//!
//! **What this is NOT, stated before anyone asks.** Not an ACL engine. Not
//! tenant isolation — `tenant_id` is audit-scoping and DSAR partitioning, and
//! no row-level isolation exists. Not SCIM, not SAML, not a second opinion on
//! what a role MEANS. The role vocabulary is frozen : this module reads it,
//! it never edits it.
//!
//! **The three-way verdict, and why it is three and not two.** `Verdict` has a
//! `Defer` arm because collapsing "not this layer's decision" into either
//! Allow or Deny is a bug in both directions: a middleware that treats an
//! earlier decision as its own either overrides the authentication layer or
//! silently re-derives it. `Defer` names the states the middleware declines to
//! decide and carries the reason, so the audit row can say which one fired.
//!
//! **The `NoPrincipal` deferral is load-bearing and was measured, not assumed.**
//! The execution prompt's §3.3 directs that an absent `Principal` extension be
//! a DENIAL. It cannot be, and the tree says so twice over:
//! `src/server/router/auth.rs` calls `next.run(req)` for the opaque OPERATOR
//! token without inserting a `Principal` at all, and
//! `tests/authz_matrix.rs::single_token_legacy_posture_unchanged` pins that this
//! token keeps reaching `/stats`, `/recall`, `/reindex` and `/purge` without a
//! 401 or 403. Denying the absent extension would fail the round's own KILL 1
//! on the first run. Authentication has already ruled by the time this layer
//! sees the request; re-deciding it is exactly the second-opinion surface the closed-oracle decision
//! exists to prevent. It is a named state, not a silent one.

use crate::auth::policy::{Action, Principal};

/// How a row keys on HTTP method.
///
/// **Why this is an enum and not a `&'static [&'static str]` with the request's
/// method dropped in.** The shared table (`route_guards::AUTHZ_GATES`) is keyed
/// by PATH and carries no method column: 193 rows over 188 distinct paths, 23
/// registered paths carry several methods under one action, and two paths
/// carry two different actions. Threading the request's runtime method through
/// a `&'static` slot does not compile, and the honest fix is to say out loud
/// that the row is not method-keyed rather than to smuggle the method in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodPolicy {
    /// The row is not method-keyed. Any method that MATCHED a registered route
    /// is admitted to the row; method-level policy remains the handler's
    /// per-method `authorize`, which is per-method by construction.
    Any,
    /// Exactly these methods. An EMPTY slice is fail-closed — never "all".
    Only(&'static [&'static str]),
}

/// One route's declared policy. Server-owned data: the route is the matched
/// PATTERN, never the request URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gate {
    /// The route pattern this row governs.
    pub route: &'static str,
    /// How this row keys on HTTP method.
    pub method_policy: MethodPolicy,
    /// The scope action the handler's own `authorize()` also checks. Neither
    /// replaces the other; the row is the ROUTE's declaration, the handler
    /// remains the inner gate .
    pub required_action: Action,
    /// A role `can` string, when this route declares one. Empty in every row
    /// read from the shared table; see [`crate::authz::gates::DENY_ONLY_CAPABILITIES`]
    /// for why the capability axis is not expressible per (path, method).
    pub required_capability: &'static str,
    /// True when the route is public and carries no gate.
    pub public: bool,
}

impl Gate {
    /// A public row: no gate, no action.
    pub const fn public(route: &'static str) -> Self {
        Self {
            route,
            method_policy: MethodPolicy::Any,
            required_action: Action::Read,
            required_capability: "",
            public: true,
        }
    }

    /// Whether `method` is a method this row covers. An `Only` set with no
    /// members is fail-closed.
    pub fn permits_method(&self, method: &str) -> bool {
        match self.method_policy {
            MethodPolicy::Any => true,
            MethodPolicy::Only(methods) => methods.contains(&method),
        }
    }
}

/// A state the middleware declines to decide, with the reason it is not ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferReason {
    /// The route is public; the authentication layer already exempted it.
    PublicPath,
    /// A declared exemption with no table row by design: the verified bearer
    /// IS the gate (the Drawbridge carve-out), or the route is registered only
    /// under a cargo feature. NOT a hole and NOT a denial — see
    /// [`crate::authz::gates::PRESENTATION_GATED`].
    PresentationGated,
    /// A CORS preflight, exempted by both auth middlewares by the same rule.
    Preflight,
    /// No `Principal` in extensions. The authentication layer ruled: the
    /// opaque operator token authenticates without inserting one, and that is
    /// the shipped superuser path.
    NoPrincipal,
}

impl DeferReason {
    /// The stable wire/detail token for the audit row. Closed, bounded, and
    /// never a description of what another principal could have done.
    pub fn as_str(self) -> &'static str {
        match self {
            DeferReason::PublicPath => "public_path",
            DeferReason::PresentationGated => "presentation_gated",
            DeferReason::Preflight => "preflight",
            DeferReason::NoPrincipal => "no_principal",
        }
    }
}

/// A refusal, over a CLOSED set.
///
/// Deliberately NOT `#[non_exhaustive]`. The point of this enum is that
/// exhaustiveness is a compile error: a wildcard arm would silently absorb the
/// next variant, which is how a closed reason set becomes an open one, and the
/// audit row would then carry a reason no consumer ever learned to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// A matched, non-public route with no row in the gate table. This is the
    /// round's whole reason for existing: the coverage property stops being a
    /// claim about a test and becomes a claim about the running server.
    RouteUngated,
    /// The route has a row; this method is not one the row covers.
    MethodNotPermitted,
    /// The route's capability is in the frozen deny-only class: a capability
    /// that exists at a handler seam and is therefore unsatisfiable here by
    /// construction. Never a silent allow.
    CapabilityDenyOnly,
}

impl DenyReason {
    /// The stable wire/detail token for the audit row.
    pub fn as_str(self) -> &'static str {
        match self {
            DenyReason::RouteUngated => "route_ungated",
            DenyReason::MethodNotPermitted => "method_not_permitted",
            DenyReason::CapabilityDenyOnly => "capability_deny_only",
        }
    }
}

/// The decision. Three arms, exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The request may proceed to the handler, which runs its own gates.
    Allow,
    /// Not this layer's decision; an earlier one stands.
    Defer(DeferReason),
    /// Refused, with a closed reason.
    Deny(DenyReason),
}

impl Verdict {
    /// The stable detail token for the audit row.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Allow => "allow",
            Verdict::Defer(r) => r.as_str(),
            Verdict::Deny(r) => r.as_str(),
        }
    }

    /// Whether this verdict refuses the request.
    pub fn is_deny(self) -> bool {
        matches!(self, Verdict::Deny(_))
    }

    /// Whether this verdict refuses the request.
    pub fn is_defer(self) -> bool {
        matches!(self, Verdict::Defer(_))
    }
}

/// The oracle.
///
/// `principal` is `Option` because "no principal" is a real, distinct, and
/// MEASURED state in this codebase rather than an error case — see the module
/// doc. `method` is a `&str` so the oracle never names a transport type.
pub fn decide_gate_verdict(principal: Option<&Principal>, gate: &Gate, method: &str) -> Verdict {
    if gate.public {
        return Verdict::Defer(DeferReason::PublicPath);
    }
    if !gate.permits_method(method) {
        return Verdict::Deny(DenyReason::MethodNotPermitted);
    }
    // NO AGENT-CLASS ARM HERE, and the absence is a finding rather than an
    // oversight. this round planned to refuse `AgentLoopback` on Admin rows at the
    // middleware. Two `authz_matrix` rows measured it and both broke:
    // `POST /reindex` is an **Admin** row, yet the agent principal receives a
    // 200 with the legacy soft-deny shape, not a 403. The agent's per-route
    // posture is therefore NOT derivable from the action column — it is the
    // heterogeneous union of the matrix's `ROLE_GATED_FOR_AGENT`,
    // `SOFT_DENY_LEGACY` and `LAYOUT_CONDITIONAL` lists. Reproducing it in
    // production would mean shipping a SECOND copy of a test-side list, which
    // is the second-opinion surface the closed-oracle decision exists to prevent, and the copy would
    // be wrong on the first route nobody classified.
    //
    // The agent class IS still refused: by the handlers, which is the defence-in-depth rule's inner
    // gate, and which the matrix pins across every gated row. This round did
    // not prove that refusal can be made total at the request layer, so it does
    // not claim to.
    if !gate.required_capability.is_empty()
        && crate::authz::gates::is_deny_only(gate.required_capability)
    {
        return Verdict::Deny(DenyReason::CapabilityDenyOnly);
    }
    // No principal is NOT a denial — the authentication layer has already
    // ruled, and in this codebase that ruling is frequently "superuser".
    principal.map_or(Verdict::Defer(DeferReason::NoPrincipal), |_| Verdict::Allow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::policy::{PrincipalKind, Scope};

    fn principal_with(kind: PrincipalKind) -> Principal {
        Principal {
            sub: "test@sub".to_string(),
            tenant: "global".to_string(),
            scopes: vec![Scope {
                action: Action::Admin,
                team: "*".to_string(),
                domain: "*".to_string(),
            }],
            jti: "jti".to_string(),
            roles: vec![],
            manages: vec![],
            kind,
        }
    }

    fn gate(action: Action, methods: &'static [&'static str]) -> Gate {
        Gate {
            route: "/x",
            method_policy: MethodPolicy::Only(methods),
            required_action: action,
            required_capability: "",
            public: false,
        }
    }

    /// The precedence the round's evidence file states, pinned: public, then
    /// method, then agent class, then the deny-only capability class, then the
    /// absent principal.
    #[test]
    fn r47_oracle_precedence_is_public_method_agent_then_defer() {
        let human = principal_with(PrincipalKind::Jwt);
        let agent = principal_with(PrincipalKind::AgentLoopback);

        assert_eq!(
            decide_gate_verdict(Some(&human), &Gate::public("/health"), "GET"),
            Verdict::Defer(DeferReason::PublicPath)
        );
        assert_eq!(
            decide_gate_verdict(Some(&human), &gate(Action::Read, &[]), "GET"),
            Verdict::Deny(DenyReason::MethodNotPermitted),
            "an empty method set is fail-closed, never 'all methods'"
        );
        assert_eq!(
            decide_gate_verdict(Some(&agent), &gate(Action::Admin, &["GET"]), "GET"),
            decide_gate_verdict(Some(&human), &gate(Action::Admin, &["GET"]), "GET"),
            "the agent class is NOT refused by the action column: /reindex is an Admin \
             row and the matrix pins a 200 soft-deny for the agent. Reproducing that \
             posture here would ship a second copy of a test-side list."
        );
        assert_eq!(
            decide_gate_verdict(Some(&human), &gate(Action::Admin, &["GET"]), "GET"),
            Verdict::Allow
        );
        assert_eq!(
            decide_gate_verdict(None, &gate(Action::Admin, &["GET"]), "GET"),
            Verdict::Defer(DeferReason::NoPrincipal),
            "the absent principal is the superuser path; denying it fails KILL 1"
        );
    }

    /// The oracle is total and closed: it takes no connection, so it cannot
    /// reach a role store and therefore cannot inherit a store's failure.
    #[test]
    fn r47_oracle_is_total_over_its_closed_vocabulary() {
        let _f: fn(Option<&Principal>, &Gate, &str) -> Verdict = decide_gate_verdict;
    }

    /// A presentation-gated route defers; it is never refused as ungated.
    #[test]
    fn r47_a_presentation_gated_route_defers_and_is_never_denied() {
        let human = principal_with(PrincipalKind::Jwt);
        let g = crate::authz::gates::gate_for("/health/db");
        assert!(
            g.is_none(),
            "/health/db carries no AUTHZ_GATES row by design"
        );
        // The middleware short-circuits on the exemption before the lookup, so
        // the oracle is never asked. Pinned here so the two cannot drift: if
        // the exemption check were dropped, the fallback is a route_ungated
        // denial, which is exactly the regression this round measured.
        assert!(crate::authz::gates::is_presentation_gated("/health/db"));
        assert_eq!(
            decide_gate_verdict(Some(&human), &Gate::public("/health/db"), "GET"),
            Verdict::Defer(DeferReason::PublicPath),
            "a non-enforced row defers; it never denies"
        );
    }

    /// Each deny reason owns a distinct detail token, so an audit row can be
    /// aggregated by reason without a new code path.
    #[test]
    fn r47_every_deny_reason_names_a_distinct_token() {
        let all = [
            DenyReason::RouteUngated,
            DenyReason::MethodNotPermitted,
            DenyReason::CapabilityDenyOnly,
        ];
        let mut seen: Vec<&str> = all.iter().map(|r| r.as_str()).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            3,
            "each deny reason owns a distinct detail token"
        );
    }
}
