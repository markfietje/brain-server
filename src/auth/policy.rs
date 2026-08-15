//! Authorization primitives (v1.2.0 "AuthN" M3).
//!
//! The hot path is `is_authorized`, which is O(scopes.len()) per request —
//! typically 1–10 scopes, so tens of nanoseconds. No DB lookup, no network
//! call: the token IS the source of truth (its claims drive the principal),
//! revocation is the safety net.
//!
//! Scope syntax: `<action>:<team>/<domain>` where `<team>` and `<domain>`
//! may be `*` (wildcard). Examples:
//!   - `read:team-alpha/*`        read any domain in team-alpha
//!   - `write:team-alpha/l1`      write the l1 domain in team-alpha
//!   - `admin:*/*`                superuser across every team+domain
//!
//! Escalation: `write` implies `read` down (a writer can read), `admin`
//! implies both. This matches least-privilege reality.
//!
//! Default-deny: no matching scope → 403. We return 403 (not 404) for
//! existence-leak reasons (OWASP A01:2025): "no such domain" vs "you can't
//! see this domain" leaks whether a domain exists.
//!
//! ponytail ceiling: the v1.2 surface is a pure function (`is_authorized`).
//! The OPA/Cedar trait (for v2.1+ distributed policy evaluation) is NOT
//! shipped — YAGNI until a real deployment needs it. Adding it later is one
//! trait definition; the scope-matching logic stays unchanged.

/// What the caller is trying to do. Maps to the route enforcement matrix in
/// `IMPLEMENTATION_PLAN_v1.2.0_AuthN.md` §3.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Read,
    Write,
    Admin,
    /// Cross-domain graph traversal. Distinct from Read so a principal can be
    /// granted traversal without broad read (e.g. an integration that walks
    /// the entity graph but doesn't see chunk content).
    Traverse,
}

impl Action {
    /// The privilege level of this action. Used for escalation checks: a
    /// `write` scope satisfies a `read` action because write is strictly
    /// stronger.
    fn rank(self) -> u8 {
        match self {
            Action::Read => 0,
            Action::Traverse => 0,
            Action::Write => 1,
            Action::Admin => 2,
        }
    }
}

/// An authenticated principal. Built from a verified JWT's claims in the
/// middleware; injected into request extensions. The `Option<Principal>`
/// pattern in handlers means `None` = opaque-token mode or no auth (the
/// v1.1 back-compat path: superuser, all scopes implicit).
#[derive(Debug, Clone)]
pub struct Principal {
    pub sub: String,
    /// OWASP Multi-Tenant: the tenant this principal belongs to. Read by
    /// audit-log scoping + cross-tenant AuthZ checks.
    pub tenant: String,
    pub scopes: Vec<Scope>,
    /// The `jti` of the access token this principal came from. Used for
    /// audit attribution (who did what, with which token).
    pub jti: String,
    /// v1.23.0 "Roles": the role *names* from the JWT `roles` claim. Empty =
    /// no role layer (the v1.14 scope path applies unchanged — back-compat).
    pub roles: Vec<String>,
    /// v1.23.0 "Roles": the `manages` claim (their direct reports / agents),
    /// the source for an `owner_filter: "reports"` role's record gate. Empty =
    /// no reports (a reports-role sees nothing by default — deny-by-default).
    pub manages: Vec<String>,
}

/// A parsed scope. `<action>:<team>/<domain>`. Lowercased on parse so
/// comparison is case-insensitive (matches the `is_valid_domain` rule that
/// domain names are lowercase).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub action: Action,
    pub team: String,
    pub domain: String,
}

impl Scope {
    /// Parse `read:team-alpha/l1` into a typed Scope. Returns None on any
    /// shape error — callers silently drop unparseable scopes (a misformed
    /// scope grants nothing, which is the safe default).
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let (action_part, rest) = raw.split_once(':')?;
        let action = match action_part.trim().to_ascii_lowercase().as_str() {
            "read" => Action::Read,
            "write" => Action::Write,
            "admin" => Action::Admin,
            "traverse" => Action::Traverse,
            // `*` as an action means admin (superuser scope).
            "*" => Action::Admin,
            _ => return None,
        };
        let (team, domain) = rest.split_once('/')?;
        let team = team.trim().to_ascii_lowercase();
        let domain = domain.trim().to_ascii_lowercase();
        if team.is_empty() || domain.is_empty() {
            return None;
        }
        Some(Scope {
            action,
            team,
            domain,
        })
    }

    /// True when this scope grants the requested action on (team, domain).
    /// Wildcards in either field match anything.
    fn grants(&self, action: Action, team: &str, domain: &str) -> bool {
        let action_ok = self.action.rank() >= action.rank();
        let team_ok = self.team == "*" || self.team == team;
        let domain_ok = self.domain == "*" || self.domain == domain;
        action_ok && team_ok && domain_ok
    }
}

/// Convenience: a principal is authorized if any of its scopes grants the
/// (action, team, domain) tuple. Used by `handlers::authorize` which wraps this
/// with the `Option<Principal>` back-compat path.
///
/// Audit G2 (v1.11.0): an authenticated principal with zero valid scopes is
/// deny-all — a token that carried no grants grants nothing. Explicit
/// superuser requires `admin:*/*` (the `*:*/*` scope). The `None`-principal
/// path (opaque-token/no-JWT back-compat) stays superuser in
/// `handlers::authorize`.
pub fn is_authorized(principal: &Principal, action: Action, team: &str, domain: &str) -> bool {
    let team_lc = team.to_ascii_lowercase();
    let domain_lc = domain.to_ascii_lowercase();
    principal
        .scopes
        .iter()
        .any(|s| s.grants(action, &team_lc, &domain_lc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_parsing_round_trips() {
        let s = Scope::parse("read:team-alpha/l1").unwrap();
        assert_eq!(s.action, Action::Read);
        assert_eq!(s.team, "team-alpha");
        assert_eq!(s.domain, "l1");
    }

    #[test]
    fn scope_parsing_rejects_garbage() {
        assert!(Scope::parse("nonsense").is_none());
        assert!(Scope::parse("read:team").is_none());
        assert!(Scope::parse("read:/l1").is_none());
        assert!(Scope::parse("read:team/").is_none());
        assert!(Scope::parse("fly:team/l1").is_none());
    }

    #[test]
    fn wildcard_team_matches_any_team() {
        let s = Scope::parse("read:*/l1").unwrap();
        assert!(s.grants(Action::Read, "any-team", "l1"));
        assert!(s.grants(Action::Read, "other-team", "l1"));
        assert!(!s.grants(Action::Read, "any-team", "other-domain"));
    }

    #[test]
    fn wildcard_domain_matches_any_domain() {
        let s = Scope::parse("read:team/*").unwrap();
        assert!(s.grants(Action::Read, "team", "anything"));
        assert!(!s.grants(Action::Read, "other-team", "anything"));
    }

    #[test]
    fn admin_star_star_is_superuser_scope() {
        let s = Scope::parse("*:*/*").unwrap();
        assert_eq!(s.action, Action::Admin);
        assert!(s.grants(Action::Read, "any", "any"));
        assert!(s.grants(Action::Write, "any", "any"));
        assert!(s.grants(Action::Admin, "any", "any"));
    }

    #[test]
    fn write_implies_read_down() {
        let s = Scope::parse("write:team/l1").unwrap();
        assert!(s.grants(Action::Read, "team", "l1"), "writer can read");
        assert!(s.grants(Action::Write, "team", "l1"));
        assert!(!s.grants(Action::Admin, "team", "l1"), "writer can't admin");
    }

    #[test]
    fn admin_implies_read_and_write() {
        let s = Scope::parse("admin:team/l1").unwrap();
        assert!(s.grants(Action::Read, "team", "l1"));
        assert!(s.grants(Action::Write, "team", "l1"));
        assert!(s.grants(Action::Admin, "team", "l1"));
    }

    #[test]
    fn empty_scopes_principal_is_deny_all_not_superuser() {
        // Audit G2 (v1.11.0): an authenticated principal with zero scopes must
        // NOT be a superuser — a token that carried no grants grants nothing.
        // Explicit superuser requires `admin:*/*` (the `*:*/*` scope).
        let p = Principal {
            sub: "op".to_string(),
            tenant: "global".to_string(),
            scopes: vec![],
            jti: String::new(),
            roles: vec![],
            manages: vec![],
        };
        assert!(!is_authorized(&p, Action::Read, "any", "any"));
        assert!(!is_authorized(&p, Action::Admin, "any", "any"));
        // The explicit superuser scope still works.
        let admin = Principal {
            sub: "op".to_string(),
            tenant: "global".to_string(),
            scopes: vec![Scope::parse("*:*/*").unwrap()],
            jti: String::new(),
            roles: vec![],
            manages: vec![],
        };
        assert!(is_authorized(&admin, Action::Admin, "any", "any"));
    }

    #[test]
    fn cross_tenant_read_is_denied() {
        let p = Principal {
            sub: "user:a".to_string(),
            tenant: "team-alpha".to_string(),
            scopes: vec![Scope::parse("read:team-alpha/*").unwrap()],
            jti: "jti-1".to_string(),
            roles: vec![],
            manages: vec![],
        };
        assert!(is_authorized(&p, Action::Read, "team-alpha", "any"));
        assert!(!is_authorized(&p, Action::Read, "team-beta", "any"));
    }
}
