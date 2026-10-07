//! Authorization primitives.
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
/// the AuthN plan §3.3.
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
            // Traverse shares Read's RANK so strictly stronger actions
            // (read/write/admin) satisfy a Traverse gate — but grants() below
            // gives a Traverse SCOPE exact-kind matching: it can never satisfy
            // Read (the enum-doc contract, honored since the 2026-09-11 fix —
            // the old rank-only check let `traverse:` scopes read full chunk
            // content, silently contradicting the documented intent).
            Action::Traverse => 0,
            Action::Write => 1,
            Action::Admin => 2,
        }
    }
}

/// What kind of credential produced this principal.
/// `Jwt` covers every verified-JWT bearer (the only principal class before
/// the opaque agent split); `AgentLoopback` is the opaque agent token resolved from the
/// token file's second line / `AGENT_TOKEN_FILE` — the typed half of the
/// operator/agent split. The class is metadata: authorization runs through
/// the SAME scope/role machinery for both (the existing matrix binds the
/// agent); the kind exists so a surface can name the class without
/// re-deriving it from scope shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    Jwt,
    AgentLoopback,
}

/// The agent principal's identity anchor (`Principal::sub`). Blackout's
/// kill-switch binds this name: revoking `agent@loopback` through
/// `POST /ops/agents/revoke` kills every agent bearer identity-wide.
pub const AGENT_LOOPBACK_SUB: &str = "agent@loopback";

/// The opaque operator superuser's identity label — the audit actor, the
/// owner-stamp string, and the `principal_label` of `Principal::None`.
/// RESERVED from the kill-switch: the operator bearer is a static token
/// with no revocable principal id, so `/ops/agents/revoke` REFUSES this
/// label loudly instead of writing an inert revocation row; the
/// remedy for a leaked operator token is rotation + restart. A JWT `sub`
/// that collides with this string inherits the refusal — it already
/// collides with the superuser's audit identity.
pub const OPERATOR_LOOPBACK_LABEL: &str = "loopback";

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
    /// the role *names* from the JWT `roles` claim. Empty =
    /// no role layer (the v1.14 scope path applies unchanged — back-compat).
    pub roles: Vec<String>,
    /// the `manages` claim (their direct reports / agents),
    /// the source for an `owner_filter: "reports"` role's record gate. Empty =
    /// no reports (a reports-role sees nothing by default — deny-by-default).
    pub manages: Vec<String>,
    /// the credential class that produced this principal.
    pub kind: PrincipalKind,
}

impl Principal {
    /// THE agent principal (Twokeys): the single identity every agent
    /// bearer authenticates as. Fixed non-Admin scope/role set —
    /// `write:*/global` (write implies read down; the shared pool only; no
    /// Admin anywhere) + the ship-with `agent` preset role (can
    /// read/write/reject: the MCP read tools + proposal writes under the
    /// review posture; NO approve/promote). The EXISTING authz matrix binds
    /// it from here — nothing agent-specific is granted anywhere.
    ///
    /// ponytail (plan non-goals): one agent identity, not per-agent
    /// identities; fine-grained agent tokens wait for a real second
    /// consumer.
    ///
    /// this round CORRECTION: this doc used to say the workflow-engine capability
    /// (`workflow`) was NOT grantable to any preset role, because the
    /// `can` allowlist is restricted to CAN_ACTIONS and — the old text
    /// claimed — does not list that verb. That was FALSE: CAN_ACTIONS names
    /// `workflow` (`src/role.rs`) and the shipped `workflow-operator` preset
    /// grants exactly `can:["workflow"]`. The agent is nonetheless refused on
    /// the workflow surfaces, but for the correct and much more boring reason:
    /// the agent's own preset role (`agent`) holds
    /// `can:["read","write","reject"]`, which does not include it. Engine
    /// surfaces stay operator-side; the ceiling is the agent ROLE, not the
    /// verb. `r47_no_comment_claims_workflow_is_ungrantable` holds it.
    pub fn agent_loopback() -> Self {
        Principal {
            sub: AGENT_LOOPBACK_SUB.to_string(),
            tenant: "global".to_string(),
            scopes: vec![Scope {
                action: Action::Write,
                team: "*".to_string(),
                domain: "global".to_string(),
            }],
            jti: "loopback-agent".to_string(),
            roles: vec!["agent".to_string()],
            manages: vec![],
            kind: PrincipalKind::AgentLoopback,
        }
    }

    /// The Twokeys agent principal over an EXPLICIT domain set: same
    /// identity, same fixed role, same no-Admin ceiling — the domain leg of
    /// the scope vec carries one `write:*/<d>` per entry (Write implies
    /// Read down). Empty/invalid input collapses to the `global`-only
    /// posture, so an unparsable grant set can never widen by accident.
    pub fn agent_loopback_for_domains(domains: &[String]) -> Self {
        let mut p = Self::agent_loopback();
        let mut scopes: Vec<Scope> = domains
            .iter()
            .map(|d| d.trim().to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .take(256)
            .map(|domain| Scope {
                action: Action::Write,
                team: "*".to_string(),
                domain,
            })
            .collect();
        if scopes.is_empty() {
            scopes.push(Scope {
                action: Action::Write,
                team: "*".to_string(),
                domain: "global".to_string(),
            });
        }
        p.scopes = scopes;
        p
    }
}

/// Env knob listing the agent principal's domains, comma-separated
/// (`BRAIN_AGENT_DOMAINS=global,gutmindsynergy`). When unset or empty the
/// middleware auto-detects: the distinct domains present in `knowledge`.
pub const AGENT_DOMAINS_ENV: &str = "BRAIN_AGENT_DOMAINS";

/// The agent principal's domain set for this request: the env list when
/// configured, otherwise the distinct domains currently in `knowledge`
/// (auto-detection). Detection errors and an empty table both fall back to
/// `global` only — the failure mode is the pre-split ceiling, never a
/// silent grant.
pub fn agent_domains(conn: &rusqlite::Connection) -> Vec<String> {
    if let Ok(raw) = std::env::var(AGENT_DOMAINS_ENV) {
        let list: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .take(256)
            .collect();
        if !list.is_empty() {
            return list;
        }
    }
    let found: Result<Vec<String>, rusqlite::Error> = (|| {
        let mut stmt = conn.prepare("SELECT DISTINCT domain FROM knowledge")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let d = row?.trim().to_ascii_lowercase();
            if !d.is_empty() {
                out.push(d);
            }
        }
        Ok(out)
    })();
    match found {
        Ok(mut list) if !list.is_empty() => {
            list.sort();
            list.dedup();
            list
        }
        _ => vec!["global".to_string()],
    }
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
    /// Team wildcards match any team; a DOMAIN wildcard (`*`) grants only the
    /// shared `global` pool — domains are a flat namespace with no tenant
    /// qualification, so a `read:<team>/*` scope reading EVERY tenant's
    /// domain would be a cross-tenant grant the team field can never narrow
    /// (the caller's own team is pinned at every check site). Granting a
    /// specific domain requires naming it: `read:<team>/acme-us`. This
    /// matches `client_authorized_domains`, which already strips `*`/`global`.
    fn grants(&self, action: Action, team: &str, domain: &str) -> bool {
        // Traverse is a DISTINCT permission, not a rank peer of Read: a
        // `traverse:` scope grants exactly Traverse (an integration that
        // walks the graph must not thereby read chunk content), while
        // strictly stronger scopes (read/write/admin) still satisfy a
        // Traverse gate (they outrank it by rank).
        let action_ok = if self.action == Action::Traverse {
            action == Action::Traverse
        } else {
            self.action.rank() >= action.rank()
        };
        // The total grant (`*/*`) needs the explicit admission: without
        // BRAIN_ALLOW_WILDCARD_GRANT=1 it grants nothing (fail closed).
        // A wildcard team over a NAMED domain keeps its prior meaning.
        let total_wildcard = self.team == "*" && self.domain == "*";
        let team_ok = self.team == team
            || (self.team == "*" && !total_wildcard)
            || (total_wildcard && crate::config::allow_wildcard_grant().unwrap_or(false));
        let domain_ok = self.domain == domain
            || (self.domain == "*" && (domain == "global" || self.team == "*"));
        action_ok && team_ok && domain_ok
    }
}

/// Convenience: a principal is authorized if any of its scopes grants the
/// (action, team, domain) tuple. Used by `handlers::authorize` which wraps this
/// with the `Option<Principal>` back-compat path.
///
/// An authenticated principal with zero valid scopes is
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

/// the client-domain allowlist seam. A `client-auditor`
/// principal (a client's compliance login) is granted exactly the client
/// domains its scopes name — the non-wildcard `domain` of each scope is the
/// allowlist key (e.g. scope `admin:ops/acme-us` → client-domain `acme-us`).
/// `None` = unrestricted: not a client-auditor, or the opaque/loopback
/// back-compat superuser. `Some(&[])` = a client-auditor with no granted
/// client-domain sees nothing (deny-by-default). The operator's `global` root
/// is never a valid auditor grant (the min-necessary wedge never widens to the
/// operator pool). The operator binds the auditor's JWT `scopes` to that
/// client's domain; the register (not this key) remains the source of truth
/// for which client-domain maps to which row.
pub fn client_authorized_domains(principal: &Option<Principal>) -> Option<Vec<String>> {
    let p = principal.as_ref()?;
    if !p.roles.iter().any(|r| r == "client-auditor") {
        return None;
    }
    Some(
        p.scopes
            .iter()
            .map(|s| s.domain.clone())
            .filter(|d| d != "*" && d != "global")
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    static SCOPE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn restore_env(key: &str, prev: Option<String>) {
        if let Some(v) = prev {
            unsafe { std::env::set_var(key, v) };
        } else {
            unsafe { std::env::remove_var(key) };
        }
    }

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
    fn wildcard_domain_grants_only_the_shared_pool() {
        // Narrowed from "matches any domain": a single-team wildcard grant
        // over a FLAT namespace would read every tenant's domains, and the
        // team field can never narrow it (callers pin their own team).
        let s = Scope::parse("read:team/*").unwrap();
        assert!(s.grants(Action::Read, "team", "global"));
        assert!(!s.grants(Action::Read, "team", "acme-us"));
        assert!(!s.grants(Action::Read, "other-team", "global"));
    }

    #[test]
    fn admin_star_star_is_superuser_scope() {
        let _guard = SCOPE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("BRAIN_ALLOW_WILDCARD_GRANT").ok();
        unsafe { std::env::set_var("BRAIN_ALLOW_WILDCARD_GRANT", "1") };
        let s = Scope::parse("*:*/*").unwrap();
        assert_eq!(s.action, Action::Admin);
        assert!(s.grants(Action::Read, "any", "any"));
        assert!(s.grants(Action::Write, "any", "any"));
        assert!(s.grants(Action::Admin, "any", "any"));
        restore_env("BRAIN_ALLOW_WILDCARD_GRANT", prev);
    }

    #[test]
    fn total_wildcard_refuses_without_admission() {
        let _guard = SCOPE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("BRAIN_ALLOW_WILDCARD_GRANT").ok();
        unsafe { std::env::remove_var("BRAIN_ALLOW_WILDCARD_GRANT") };
        let s = Scope::parse("admin:*/*").unwrap();
        assert!(!s.grants(Action::Read, "any", "any"));
        assert!(!s.grants(Action::Admin, "any", "global"));
        restore_env("BRAIN_ALLOW_WILDCARD_GRANT", prev);
    }

    #[test]
    fn write_implies_read_down() {
        let s = Scope::parse("write:team/l1").unwrap();
        assert!(s.grants(Action::Read, "team", "l1"), "writer can read");
        assert!(s.grants(Action::Write, "team", "l1"));
        assert!(!s.grants(Action::Admin, "team", "l1"), "writer can't admin");
    }

    #[test]
    fn traverse_scope_grants_only_traverse() {
        // 2026-09-11 fix pin: a `traverse:` scope NEVER satisfies a Read gate
        // (the enum-doc contract — an integration that walks the graph must
        // not thereby read chunk content), while read outranks traverse.
        let s = Scope::parse("traverse:team/l1").unwrap();
        assert!(s.grants(Action::Traverse, "team", "l1"));
        assert!(
            !s.grants(Action::Read, "team", "l1"),
            "traverse must never satisfy Read"
        );
        assert!(!s.grants(Action::Write, "team", "l1"));
        let r = Scope::parse("read:team/l1").unwrap();
        assert!(
            r.grants(Action::Traverse, "team", "l1"),
            "read outranks traverse"
        );
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
        // Audit G2: an authenticated principal with zero scopes must
        // NOT be a superuser — a token that carried no grants grants nothing.
        // Explicit superuser requires `admin:*/*` (the `*:*/*` scope).
        let p = Principal {
            sub: "op".to_string(),
            tenant: "global".to_string(),
            scopes: vec![],
            jti: String::new(),
            roles: vec![],
            manages: vec![],
            kind: PrincipalKind::Jwt,
        };
        assert!(!is_authorized(&p, Action::Read, "any", "any"));
        assert!(!is_authorized(&p, Action::Admin, "any", "any"));
        // The explicit superuser scope still works (under admission).
        let _guard = SCOPE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("BRAIN_ALLOW_WILDCARD_GRANT").ok();
        unsafe { std::env::set_var("BRAIN_ALLOW_WILDCARD_GRANT", "1") };
        let admin = Principal {
            sub: "op".to_string(),
            tenant: "global".to_string(),
            scopes: vec![Scope::parse("*:*/*").unwrap()],
            jti: String::new(),
            roles: vec![],
            manages: vec![],
            kind: PrincipalKind::Jwt,
        };
        assert!(is_authorized(&admin, Action::Admin, "any", "any"));
        restore_env("BRAIN_ALLOW_WILDCARD_GRANT", prev);
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
            kind: PrincipalKind::Jwt,
        };
        // A domain wildcard grants the shared pool…
        assert!(is_authorized(&p, Action::Read, "team-alpha", "global"));
        // …not other tenants' named domains (flat namespace: the team field
        // can never narrow a `*` domain grant)…
        assert!(!is_authorized(&p, Action::Read, "team-alpha", "acme-us"));
        assert!(!is_authorized(&p, Action::Read, "team-beta", "global"));
        // …and naming a domain requires a scope that names it.
        let named = Principal {
            scopes: vec![
                Scope::parse("read:team-alpha/*").unwrap(),
                Scope::parse("read:team-alpha/acme-us").unwrap(),
            ],
            ..p
        };
        assert!(is_authorized(&named, Action::Read, "team-alpha", "acme-us"));
    }

    #[test]
    fn explicit_superuser_still_grants_named_domains() {
        // `admin:*/*` is the documented explicit-superuser shape: both fields
        // wildcarded grants everything (under the wildcard admission).
        // Narrowing single-team wildcards must not touch it.
        let _guard = SCOPE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("BRAIN_ALLOW_WILDCARD_GRANT").ok();
        unsafe { std::env::set_var("BRAIN_ALLOW_WILDCARD_GRANT", "1") };
        let p = Principal {
            sub: "user:root".to_string(),
            tenant: "global".to_string(),
            scopes: vec![Scope::parse("admin:*/*").unwrap()],
            jti: "jti-2".to_string(),
            roles: vec![],
            manages: vec![],
            kind: PrincipalKind::Jwt,
        };
        for domain in ["global", "acme-us", "beta-eu"] {
            for team in ["team-alpha", "team-beta"] {
                assert!(is_authorized(&p, Action::Read, team, domain));
                assert!(is_authorized(&p, Action::Write, team, domain));
            }
        }
        restore_env("BRAIN_ALLOW_WILDCARD_GRANT", prev);
    }

    fn auditor(roles: &[&str], scopes: &[&str]) -> Option<Principal> {
        Some(Principal {
            sub: "audit".to_string(),
            tenant: "global".to_string(),
            scopes: scopes.iter().filter_map(|s| Scope::parse(s)).collect(),
            jti: String::new(),
            roles: roles.iter().map(|s| s.to_string()).collect(),
            manages: vec![],
            kind: PrincipalKind::Jwt,
        })
    }

    #[test]
    fn client_authorized_domains_is_restricted_only_for_client_auditor() {
        let p = auditor(&[], &["admin:ops/acme-us"]);
        assert!(
            client_authorized_domains(&p).is_none(),
            "non-auditor: unrestricted"
        );
        assert!(
            client_authorized_domains(&None).is_none(),
            "no principal (loopback/opaque): unrestricted"
        );
    }

    #[test]
    fn client_authorized_domains_deny_by_default_when_only_wildcard_or_global() {
        // A client-auditor with only the operator's wildcard/global scopes gets
        // Some(&[]) -> sees nothing. The min-necessary wedge never widens to the
        // operator pool. ponytail: covers the no-global/no-wildcard invariant.
        let p = auditor(&["client-auditor"], &["admin:*/*", "read:ops/global"]);
        assert_eq!(
            client_authorized_domains(&p),
            Some(vec![]),
            "wildcard + global grant no client domain (deny-by-default)"
        );
    }

    #[test]
    fn client_authorized_domains_lists_only_concrete_non_global_domains() {
        let p = auditor(
            &["client-auditor"],
            &[
                "read:ops/acme-us",
                "read:ops/*",
                "admin:ops/global",
                "write:ops/beta-eu",
            ],
        );
        let got = client_authorized_domains(&p).unwrap();
        assert_eq!(
            got,
            vec!["acme-us", "beta-eu"],
            "only concrete, non-global domains"
        );
        assert!(!got.iter().any(|d| d == "*" || d == "global"));
    }

    #[test]
    fn agent_principal_over_an_empty_domain_set_stays_global_only() {
        let p = Principal::agent_loopback_for_domains(&[]);
        assert_eq!(p.scopes.len(), 1);
        assert_eq!(p.scopes[0].domain, "global");
        assert_eq!(p.scopes[0].action, Action::Write);
    }

    #[test]
    fn agent_principal_over_named_domains_writes_each_and_reads_down() {
        let p = Principal::agent_loopback_for_domains(&[
            "GutMindSynergy".to_string(),
            " global ".to_string(),
            "gutmindsynergy".to_string(),
        ]);
        // dedup is the caller's job; lowercasing is this constructor's.
        assert!(
            p.scopes
                .iter()
                .any(|s| s.domain == "gutmindsynergy" && s.action == Action::Write)
        );
        assert!(
            p.scopes
                .iter()
                .any(|s| s.domain == "global" && s.action == Action::Write)
        );
        assert!(p.scopes.iter().all(|s| s.team == "*"));
        assert!(p.roles.iter().any(|r| r == "agent"));
        assert_eq!(p.kind, PrincipalKind::AgentLoopback);
    }

    fn memdb() -> rusqlite::Connection {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE knowledge (id INTEGER PRIMARY KEY, domain TEXT);
             INSERT INTO knowledge (domain) VALUES ('global'), ('gutmindsynergy'), ('health'), ('GutMindSynergy');",
        )
        .unwrap();
        c
    }

    #[test]
    fn agent_domains_auto_detects_distinct_knowledge_domains() {
        let _env = SCOPE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(AGENT_DOMAINS_ENV).ok();
        unsafe { std::env::remove_var(AGENT_DOMAINS_ENV) };
        let got = agent_domains(&memdb());
        restore_env(AGENT_DOMAINS_ENV, prev);
        assert!(got.contains(&"global".to_string()));
        assert!(got.contains(&"gutmindsynergy".to_string()));
        assert!(got.contains(&"health".to_string()));
        assert_eq!(
            got.iter().filter(|d| *d == "gutmindsynergy").count(),
            1,
            "lowercasing must merge the 'GutMindSynergy' case variant"
        );
    }

    #[test]
    fn agent_domains_falls_back_to_global_on_an_empty_table() {
        let _env = SCOPE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(AGENT_DOMAINS_ENV).ok();
        unsafe { std::env::remove_var(AGENT_DOMAINS_ENV) };
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE knowledge (id INTEGER PRIMARY KEY, domain TEXT);")
            .unwrap();
        let got = agent_domains(&c);
        restore_env(AGENT_DOMAINS_ENV, prev);
        assert_eq!(got, vec!["global".to_string()]);
    }

    #[test]
    fn agent_domains_env_list_wins_over_detection() {
        let _env = SCOPE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(AGENT_DOMAINS_ENV).ok();
        unsafe { std::env::set_var(AGENT_DOMAINS_ENV, "global, Job , ,PERSONAL") };
        let got = agent_domains(&memdb());
        restore_env(AGENT_DOMAINS_ENV, prev);
        assert_eq!(
            got,
            vec![
                "global".to_string(),
                "job".to_string(),
                "personal".to_string()
            ]
        );
    }
}
