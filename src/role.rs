//! a Role is a **named bundle of scopes + default panel
//! visibility + an action `can` allowlist**, reused on the *existing* scope
//! `access_scope`/`owner` mechanism ("same dashboard, scoped per role"). No new
//! data filter: a role's `scopes` map straight onto the `access_scope` WHERE
//! clause; `owner_filter` (self/reports/all) narrows by the `owner` column.
//!
//! The principal's role names come from the SSO JWT `roles` claim. The
//! role *definitions* live in the `roles` store (presets, editable) so the
//! server can enforce the data filter + the action gate server-side; the client
//! hides panels / disables buttons as defense-in-depth only (never the sole
//! gate). A principal with **no** roles claim is byte-identical to pre-v1.23
//! behavior (the back-compat invariant, test-pinned).
//!
//! ## A ROLE IS USUALLY A RESTRICTION — AND THE TIMES IT IS NOT ARE ENUMERATED
//!
//! Most roles narrow the data plane: `owner_filter` `self`/`reports` becomes
//! the `owner` half of the record gate, and the role's `scopes` become the
//! `access_scope` half. This file used to claim roles were always
//! "*additional* restrictions" (and `handlers::mod` repeated it). That was
//! FALSE for any role carrying `owner_filter: "all"`, because the shortcut
//! treated that field as an admin marker:
//!
//! ```text
//! roles.iter().any(|r| r.owner_filter == "all" || r.can("admin"))
//! ```
//!
//! which returned `{None, None}` — BOTH halves of the record gate erased — for
//! three read-only compliance presets that never needed a cross-owner read:
//! `dpo`, `client-auditor`, `bpo-ops`. A token that, with no roles, could read
//! only `sub`'s own `private` rows would, with `roles: ["dpo"]` added by an
//! IdP mapping, read every owner's `private` rows in the domain its scope
//! already admitted. The secondary effect was quieter still:
//! `handlers::recall` computes `role_restricted = owner_in.is_some()`, so the
//! cross-domain `scope_violation` audit row never fired for precisely the
//! principals that were widest.
//!
//! So the shortcut is gone. `owner_filter` narrows by the `owner` column and
//! nothing else (its documented job, `role.rs:4`), the role's `scopes` always
//! survive as `access_scopes`, and dropping the owner restriction is an
//! explicit capability — [`CAN_ACTIONS`]`'s `shared_pool` — carried only by the
//! presets in [`SHARED_POOL_ROLES`], each with the reason recorded there.
//! `owner_filter: "all"` on a role WITHOUT `shared_pool` narrows exactly like
//! `self` does, which is the fail-closed direction.
//!
//! The claim is now true as written, and
//! `only_declared_roles_widen_the_owner_fence` is the pin that keeps it true
//! when the next preset is added.
//!
//! Honest ceiling: roles are named scope bundles, not a
//! general ACL engine (per-record ownership, groups, arbitrary perms = v2.0
//! Cortex RBAC). The `reports` agent-tree needs a source (`manages` JWT claim or
//! a small table); large hierarchies may need a managed directory (v2.x SCIM).
//! The MCP `tools_allowed` field is **stored + surfaced** via
//! the role API. It is ENFORCED now at the UMP verb seam:
//! `handlers::authorize_tool` is the principal-side twin of the capability
//! `cap_gate`, called by the six UMP entry points (`ump.remember`, `ump.get`,
//! `ump.recall`, `ump.revise`, `ump.forget`, `ump.feedback`). Several roles
//! are the union of their grants; `"*"` is every tool; a role that does not
//! DECLARE the field is unrestricted (narrowing is opt-in, and the field was
//! never enforced, so an absent set must not silently disarm a role); a
//! role-less principal is untouched — the action seam governs that class.

use rusqlite::Connection;

/// Valid `access_scope` values a role may read (matches `AccessScopeFilter`).
pub const ROLE_SCOPES: &[&str] = &["private", "domain", "team"];

/// Valid `owner_filter` values.
pub const OWNER_FILTERS: &[&str] = &["self", "reports", "all"];

/// Every action capability the role `can` allowlist recognizes. These wire to
/// the `authorize_role` gate on the write/action handlers (approve/reject/purge/
/// DSAR/…). `admin` short-circuits to the unrestrictive data filter.
/// Grants the shared-pool READ: the ability to drop the record gate's
/// `owner` restriction. It is a data-plane capability, not an action verb —
/// no route calls `authorize_role(_, "shared_pool")`, because there is
/// nothing to authorize per-request; it is read once by
/// [`effective_filter`] when the record gate is composed. An empty scope
/// list plus `shared_pool` therefore matches nothing, exactly like any
/// other scopeless role.
///
/// It exists because `owner_filter` is a FIELD and the presets that need a
/// shared pool all happen to spell it `"all"` — which meant any role that
/// ever carried that string also widened, including three read-only
/// compliance presets that never needed to. `owner_filter` alone now
/// narrows by owner and nothing else; the widening is an explicit,
/// enumerated grant.
pub const CAN_ACTIONS: &[&str] = &[
    "read",
    "write",
    "approve",
    // Publication is its OWN capability, deliberately not implied by
    // `approve`: accepting a draft into memory and publishing it as an
    // external-facing article are different acts with different consequences,
    // and a role that does the first is not thereby granted the second.
    "publish",
    "reject",
    "calibrate",
    "release_quarantine",
    "dsar_export",
    "purge",
    "admin",
    "workflow",
    "shared_pool",
];

/// The operator-console panel names a role's `panels_default`/`panels_hidden`
/// reference (client visibility guidance — the server always enforces the
/// underlying data + action gates).
pub const ROLE_PANELS: &[&str] = &[
    "overview",
    "ingest",
    "recall",
    "search",
    "graph",
    "review",
    "procedures",
    "connectors",
    "security",
    "audit",
    "data",
    "subjects",
    "health",
    "system",
];

/// A role name must be URL/filename-safe (it becomes a path param + an FK key):
/// the domain charset (lowercase alnum + hyphen, 1..=63).
pub fn is_valid_role_name(name: &str) -> bool {
    crate::storage_layout::is_valid_domain(name)
}

/// One named scope/action bundle. Every vector field is `Vec` (not `Option`):
/// an empty list is meaningful (e.g. `scopes: []` = deny all non-admin reads).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Role {
    #[serde(default)]
    pub name: String,
    /// Display-only one-liner. Never interpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `access_scope` values this role may read (private/domain/team). Empty +
    /// not `admin` = nothing is readable (deny-by-default).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// `self` | `reports` (their agents, from the `manages` claim) | `all`.
    #[serde(default = "default_owner_filter")]
    pub owner_filter: String,
    /// Action allowlist (subset of [`CAN_ACTIONS`]). The `authorize_role` gate
    /// denies a held action not listed here (403).
    #[serde(default)]
    pub can: Vec<String>,
    /// Panels shown by default (client visibility; server enforces underneath).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panels_default: Option<Vec<String>>,
    /// Panels hidden unconditionally (e.g. capacity from non-admins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panels_hidden: Option<Vec<String>>,
    /// The MCP `ump.*` tools this role may invoke. Stored, surfaced, and
    /// ENFORCED at the UMP verb seam (`handlers::authorize_tool`). `None` =
    /// unrestricted (this role does not narrow itself); `Some` = exactly this
    /// set; `"*"` = every tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_allowed: Option<Vec<String>>,
}

fn default_owner_filter() -> String {
    "self".to_string()
}

impl Role {
    /// Whether this role's `can` allowlist names `cap`.
    pub fn can(&self, cap: &str) -> bool {
        self.can.iter().any(|c| c == cap)
    }

    /// Whether a held tool name is allowed (`"*"` = every tool).
    pub fn can_tool(&self, tool: &str) -> bool {
        self.tools_allowed
            .as_deref()
            .is_some_and(|t| t.iter().any(|n| n == "*" || n == tool))
    }
}

/// Validate a role bundle before it is stored (every write seam calls this).
pub fn validate(r: &Role) -> Result<(), String> {
    if !is_valid_role_name(&r.name) {
        return Err(format!(
            "invalid role name '{}' (lowercase alnum + hyphen, max 63)",
            r.name
        ));
    }
    for s in &r.scopes {
        if !ROLE_SCOPES.contains(&s.as_str()) {
            return Err(format!(
                "scopes item '{s}' invalid (must be {ROLE_SCOPES:?})"
            ));
        }
    }
    if !OWNER_FILTERS.contains(&r.owner_filter.as_str()) {
        return Err(format!(
            "owner_filter must be one of {OWNER_FILTERS:?} (got '{}')",
            r.owner_filter
        ));
    }
    for c in &r.can {
        if !CAN_ACTIONS.contains(&c.as_str()) {
            return Err(format!("can item '{c}' unknown (must be {CAN_ACTIONS:?})"));
        }
    }
    for p in r
        .panels_default
        .iter()
        .flatten()
        .chain(r.panels_hidden.iter().flatten())
    {
        if !ROLE_PANELS.contains(&p.as_str()) {
            return Err(format!("panel '{p}' unknown (must be {ROLE_PANELS:?})"));
        }
    }
    for t in r.tools_allowed.iter().flatten() {
        if t != "*" && (t.is_empty() || t.len() > 63) {
            return Err(format!("tool name '{t}' invalid"));
        }
    }
    Ok(())
}

// ── persistence (the `roles` table, global DB) ──────────────────────────────

/// Load one role by name. Fails closed on an unreadable stored bundle.
pub fn load(conn: &Connection, name: &str) -> Result<Option<Role>, String> {
    let json: Option<String> =
        match conn.query_row("SELECT json FROM roles WHERE name = ?1", [name], |r| {
            r.get(0)
        }) {
            Ok(j) => Some(j),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.to_string()),
        };
    match json {
        Some(j) => serde_json::from_str(&j)
            .map(Some)
            .map_err(|e| format!("stored role '{name}' is unreadable: {e}")),
        None => Ok(None),
    }
}

/// List every role (seeded presets + operator-defined), name-ordered.
pub fn list(conn: &Connection) -> Result<Vec<Role>, String> {
    let mut stmt = conn
        .prepare("SELECT json FROM roles ORDER BY name")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for j in rows.flatten() {
        // A corrupt row fails closed (surfaced), never silently dropped.
        out.push(serde_json::from_str(&j).map_err(|e| format!("stored role unreadable: {e}"))?);
    }
    Ok(out)
}

/// Upsert one role row (validated by the caller before storing).
pub fn upsert(conn: &Connection, r: &Role) -> Result<(), String> {
    let json = serde_json::to_string(r).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO roles(name, json) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET json = excluded.json",
        rusqlite::params![r.name, json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Resolve a principal's held role *names* to their definitions. Unknown names
/// are ignored (they contribute nothing — a typo'd claim grants nothing, which
/// is the safe default). Fails closed on an unreadable stored bundle.
pub fn resolve(conn: &Connection, names: &[String]) -> Result<Vec<Role>, String> {
    let mut out = Vec::new();
    for n in names {
        if let Some(r) = load(conn, n)? {
            out.push(r);
        }
    }
    Ok(out)
}

/// How many roles the deployment DEFINES. The dual-gate surfaces pass
/// scope-only tokens only while this is zero: a populated role store means
/// the deployment opted into role-based governance. Fails with the bare
/// storage error text; the caller maps it.
pub fn defined_count(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM roles", [], |r| r.get(0))
        .map_err(|e| e.to_string())
}

// ── the data-layer retrieval filter ─────────────────────────────────────────

/// A Comparator row-owner restriction (self/reports), mapped to a SQL
/// `WHERE k.owner IN (…)`. [`SearchFilters.owner_in`] carries it into the
/// retriever's shared predicate builder (vec0 + lex both honor it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RetrievalGate {
    /// Narrowed allowed `access_scope` set (None = unrestricted).
    pub access_scopes: Option<Vec<String>>,
    /// `WHERE k.owner IN` set (None = no owner restriction).
    pub owner_in: Option<Vec<String>>,
}

/// Compute a record-level retrieval gate from a principal's identity
/// (`sub` + `manages`) and its resolved roles. `deny-by-default`: a role with
/// no scopes and no `admin` capability reads nothing. `admin` (owner_filter
/// `all` or the `admin` capability) sees all. `self` → its own `owner`;
/// `reports` → the `manages` set (EMPTY set = deny, never unrestricted);
/// `all` without `shared_pool` and every unknown posture narrow to the
/// caller's own subject; only `shared_pool` (or `admin`) drops the owner
/// fence.
///
/// Kept dependency-free (no `auth::Principal`, no `search::SearchFilters`) so
/// it lives in the lib; the server's `handlers` apply it to the retriever.
pub fn effective_filter(sub: &str, manages: &[String], roles: &[Role]) -> RetrievalGate {
    let adminish = roles.iter().any(|r| r.can("admin"));
    if adminish {
        return RetrievalGate {
            access_scopes: None,
            owner_in: None,
        };
    }
    let mut scopes: Vec<String> = Vec::new();
    let mut owners: Vec<String> = Vec::new();
    for r in roles {
        for s in &r.scopes {
            if !scopes.contains(s) {
                scopes.push(s.clone());
            }
        }
        match r.owner_filter.as_str() {
            "reports" => {
                // An EMPTY `manages` claim adds nothing, so the allowlist stays
                // empty and the gate denies. It must NEVER fall through to
                // `None` — `None` means "no owner predicate", i.e. every
                // owner's rows (audit N1).
                for m in manages {
                    if !owners.contains(m) {
                        owners.push(m.clone());
                    }
                }
            }
            // `self`, `all` WITHOUT `shared_pool`, and any unknown posture all
            // narrow to the caller's own subject. `owner_filter` is a
            // restriction, never a grant; only the explicit `shared_pool`
            // capability widens the owner half (audit N2).
            _ => {
                if !owners.contains(&sub.to_string()) {
                    owners.push(sub.to_string());
                }
            }
        }
    }
    RetrievalGate {
        access_scopes: Some(scopes),
        // `owner_filter: "all"` widens ONLY the owner column, and only for a
        // role that declares the `shared_pool` capability. It never empties
        // `access_scopes`: that half of the record gate is the role's own
        // declared scope allowlist, and dropping it would admit an
        // `access_scope` value no role ever named. See the module header for
        // why `owner_filter` alone is not a grant and `shared_pool` is.
        owner_in: if roles.iter().any(|r| r.can("shared_pool")) {
            None
        } else {
            // An empty allowlist is `Some(vec![])` — the SQL `WHERE owner IN
            // ()`/`1 = 0` deny — not `None`. The `.filter(!is_empty)` here was
            // the critical bypass: a `reports` role with no `manages` claim
            // became unrestricted.
            Some(owners)
        },
    }
}

/// The presets permitted to drop the owner fence, with the reason each one
/// needs it. Every entry is a deliberate cross-owner read, not an accident of
/// the preset's shape, and `only_declared_roles_widen_the_owner_fence` fails
/// when a preset is added that is not on this list — which is the whole point:
/// `owner_filter` used to widen silently, so nothing stopped the next
/// least-privilege role from inheriting an Admin-scale read.
///
/// `client-auditor` is the honest case: its documented purpose is a read-only
/// view of exactly one client *domain* — cross-owner by definition, bounded by
/// the token's own scope domain and by its `access_scopes`. `dpo` and
/// `bpo-ops` are read-only *board* roles whose duties (DSAR export via the
/// `dsar_export` verb, the capacity/connector/queue/breach board) are verb
/// gates, not memory-row reads, so they read their own rows like any other
/// principal.
pub const SHARED_POOL_ROLES: &[&str] = &["admin", "solo", "controller", "agent", "client-auditor"];

// ── the 10 ship-with roles (curated from USE_CASES.md) ─────────────────

/// Ship-with roles as (name, json) — the exact bytes `migration` seeds and the
/// parse test validates. Seeded INSERT OR IGNORE so operator edits survive a
/// re-migration. `presets()` is the parsed view.
pub const PRESETS_RAW: &[(&str, &str)] = &[
    (
        "admin",
        r#"{"name":"admin","description":"Full control: every scope, every panel, every action","scopes":["private","domain","team"],"owner_filter":"all","can":["publish","read","write","approve","reject","calibrate","release_quarantine","dsar_export","purge","admin","shared_pool"],"panels_default":null,"panels_hidden":null,"tools_allowed":["*"]}"#,
    ),
    (
        "solo",
        r#"{"name":"solo","description":"SMB owner: all panels, all actions, unrestricted data (the simplest default)","scopes":["private","domain","team"],"owner_filter":"all","can":["publish","read","write","approve","reject","calibrate","release_quarantine","dsar_export","purge","admin","shared_pool"],"panels_default":["overview","ingest","recall","search","graph","review","procedures","connectors","security","audit","data","subjects","health","system"],"panels_hidden":null,"tools_allowed":["*"]}"#,
    ),
    (
        "agent",
        r#"{"name":"agent","description":"Front-line worker: reads and writes across the shared memory pool (no Admin, no approve, no reject — the review queue is not the agent's to prune), the gateway's standing identity","scopes":["private","domain","team"],"owner_filter":"all","can":["read","write","shared_pool"],"panels_default":["overview","ingest","recall","health"],"panels_hidden":["audit","subjects"],"tools_allowed":["ump.recall","ump.get","ump.feedback"]}"#,
    ),
    (
        "workflow-operator",
        r#"{"name":"workflow-operator","description":"Workflow operator: governed workflow execution without administrative or publication authority","scopes":[],"owner_filter":"self","can":["workflow"],"panels_default":["health"],"panels_hidden":null,"tools_allowed":[]}"#,
    ),
    (
        "supervisor",
        r#"{"name":"supervisor","description":"Call-center lead: sees only their agents' rows (manages claim), approves/rejects their queue","scopes":["private","domain","team"],"owner_filter":"reports","can":["publish","read","write","approve","reject","calibrate","release_quarantine","dsar_export"],"panels_default":["overview","review","recall","security","audit","data","health"],"panels_hidden":["subjects"],"tools_allowed":["ump.recall","ump.get","ump.revise","ump.feedback","ump.remember"]}"#,
    ),
    (
        "qa-specialist",
        r#"{"name":"qa-specialist","description":"QA: reads agent work + calibrates, cannot approve or purge","scopes":["private","domain","team"],"owner_filter":"reports","can":["read","calibrate"],"panels_default":["overview","review","recall","health"],"panels_hidden":["audit","subjects"],"tools_allowed":["ump.recall","ump.get","ump.feedback"]}"#,
    ),
    (
        "clinician",
        r#"{"name":"clinician","description":"Clinician: min-necessary PHI, own private memory only, read/write, no review","scopes":["private"],"owner_filter":"self","can":["read","write"],"panels_default":["overview","recall","health"],"panels_hidden":["audit","subjects","data"],"tools_allowed":["ump.recall","ump.get","ump.remember"]}"#,
    ),
    (
        "dpo",
        r#"{"name":"dpo","description":"Data Protection Officer: runs DSARs, no routine write, read + export + calibrate","scopes":["private","domain","team"],"owner_filter":"self","can":["read","dsar_export","calibrate"],"panels_default":["overview","recall","data","subjects","audit","health"],"panels_hidden":null,"tools_allowed":["ump.recall","ump.get","ump.feedback"]}"#,
    ),
    (
        "recruiter",
        r#"{"name":"recruiter","description":"Recruiter: per-candidate private memory, own + team pools, uses the review queue","scopes":["private","domain","team"],"owner_filter":"reports","can":["read","write","reject"],"panels_default":["overview","ingest","recall","review","health"],"panels_hidden":["audit","subjects"],"tools_allowed":["ump.recall","ump.get","ump.remember"]}"#,
    ),
    (
        "controller",
        r#"{"name":"controller","description":"Data Controller: daily operational control, broad actions, retention enforcement","scopes":["private","domain","team"],"owner_filter":"all","can":["publish","read","write","approve","reject","calibrate","release_quarantine","dsar_export","purge","shared_pool"],"panels_default":null,"panels_hidden":["security"],"tools_allowed":["*"]}"#,
    ),
    (
        "exec",
        r#"{"name":"exec","description":"Executive: read-only dashboards across the team, no write or destructive actions","scopes":["private","domain","team"],"owner_filter":"reports","can":["read"],"panels_default":["overview","health","security"],"panels_hidden":["audit","data","subjects"],"tools_allowed":["ump.recall"]}"#,
    ),
    // the BPO client postures. A client-auditor is READ-ONLY
    // on ONE client domain (its compliance login — `can` has only "read", the
    // min-necessary wedge); a bpo-ops is the all-clients operations read. Both
    // INSERT OR IGNORE so operator edits survive a re-migration.
    (
        "client-auditor",
        r#"{"name":"client-auditor","description":"A client's compliance team: read-only view of exactly one client domain","scopes":["private","domain","team"],"owner_filter":"all","can":["read","shared_pool"],"panels_default":["overview","health"],"panels_hidden":null,"tools_allowed":["ump.recall","ump.get"]}"#,
    ),
    (
        "bpo-ops",
        r#"{"name":"bpo-ops","description":"BPO operations: read-only capacity/connector/queue/breach board across all clients","scopes":["private","domain","team"],"owner_filter":"self","can":["read"],"panels_default":["overview","health"],"panels_hidden":["subjects"],"tools_allowed":["ump.recall"]}"#,
    ),
];

/// The parsed view of the ship-with roles (`PRESETS_RAW`), used by tests. A
/// preset that fails to parse is skipped; `all_presets_parse_and_validate`
/// pins `presets().len() == PRESETS_RAW.len()`, so a malformed preset fails
/// that test instead of panicking at parse time.
pub fn presets() -> Vec<Role> {
    PRESETS_RAW
        .iter()
        .filter_map(|(name, json)| {
            let mut r: Role = serde_json::from_str(json).ok()?;
            r.name = (*name).to_string();
            Some(r)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("db");
        conn.execute_batch(
            "CREATE TABLE roles(name TEXT PRIMARY KEY, json TEXT NOT NULL,
               created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP);",
        )
        .expect("schema");
        conn
    }

    fn manages(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn all_presets_parse_and_validate() {
        let all = presets();
        assert_eq!(all.len(), PRESETS_RAW.len(), "13 ship-with roles");
        for r in &all {
            validate(r).unwrap_or_else(|e| panic!("role {} invalid: {e}", r.name));
        }
        // the BPO client postures. A client-auditor is the
        // read-only wedge (can == ["read"]); bpo-ops is read-only too.
        let auditor = all.iter().find(|r| r.name == "client-auditor").unwrap();
        assert_eq!(
            auditor.can,
            vec!["read", "shared_pool"],
            "client-auditor is read-only + the declared shared-pool read"
        );
        let ops = all.iter().find(|r| r.name == "bpo-ops").unwrap();
        assert_eq!(ops.can, vec!["read"], "bpo-ops is read-only");
        let workflow = all.iter().find(|r| r.name == "workflow-operator").unwrap();
        assert_eq!(workflow.can, vec!["workflow"]);
        assert!(
            !all.iter()
                .find(|r| r.name == "agent")
                .unwrap()
                .can("workflow")
        );
    }

    /// The `solo` SMB role is still the simplest default: every action.
    #[test]
    fn solo_preset_is_the_every_action_default() {
        let solo = presets().into_iter().find(|r| r.name == "solo").unwrap();
        assert!(solo.can("approve") && solo.can("purge") && solo.can("dsar_export"));
    }

    /// THE GUARD AGAINST RECURRENCE. `owner_filter: "all"` widened the record
    /// gate silently for years, so a new read-only preset inheriting the string
    /// inherited an Admin-scale read with nothing to fail. The widening is now
    /// the `shared_pool` capability, and a role may carry it only if it is on
    /// the enumerated list, each entry of which records why.
    ///
    /// Red-first, three ways: restoring `|| r.owner_filter == "all"` to
    /// `effective_filter` does NOT red this (the list is what it consults) —
    /// `dpo_owner_filter_all_does_not_widen` reds instead. Adding a preset with
    /// `shared_pool` that is not on the list reds here. Adding `shared_pool` to
    /// a preset already on the list reds `shared_pool_roles_still_need_their_reason`.
    #[test]
    fn only_declared_roles_widen_the_owner_fence() {
        let all = presets();
        for r in &all {
            let widens = r.can("shared_pool");
            assert_eq!(
                widens,
                SHARED_POOL_ROLES.contains(&r.name.as_str()),
                "role {:?} shared_pool={} disagrees with SHARED_POOL_ROLES — either add it \
                 there with its reason, or drop the capability",
                r.name,
                widens
            );
        }
        // No `shared_pool` role may be read-only-but-not-one-of-the-three: the
        // capability is a READ grant, so every holder should be able to read.
        for r in &all {
            assert!(
                !r.can("shared_pool") || r.can("read"),
                "role {:?} grants shared_pool without read",
                r.name
            );
        }
    }

    /// `owner_filter` is a FIELD. The three read-only compliance roles used to
    /// carry `"all"` and therefore widened, which is the exact accident the
    /// enumeration above exists to prevent — so their filters are asserted to
    /// be owner-BOUND now, and `client-auditor` (the one role that genuinely
    /// needs a cross-owner read) is asserted to still have it.
    #[test]
    fn dpo_owner_filter_all_does_not_widen() {
        let all = presets();
        for name in ["dpo", "bpo-ops"] {
            let r = all.iter().find(|r| r.name == name).unwrap();
            let gate = effective_filter("ana", &[], std::slice::from_ref(r));
            assert!(
                gate.owner_in
                    .as_ref()
                    .is_some_and(|o| o == &vec!["ana".to_string()]),
                "{name} must stay owner-bound: {:?}",
                gate.owner_in
            );
            // and the role's scopes still bound the access_scope half
            assert_eq!(
                gate.access_scopes,
                Some(vec![
                    "private".to_string(),
                    "domain".to_string(),
                    "team".to_string()
                ]),
                "{name} keeps its declared scope allowlist"
            );
        }
        let auditor = all.iter().find(|r| r.name == "client-auditor").unwrap();
        let gate = effective_filter("ana", &[], std::slice::from_ref(auditor));
        assert_eq!(
            gate.owner_in, None,
            "client-auditor's documented job IS a cross-owner read of one domain"
        );
        assert!(
            gate.access_scopes.is_some(),
            "even a shared-pool role keeps its access_scope allowlist — that half of \
             the record gate is never erased by owner_filter"
        );
    }

    /// The `agent` preset's shared-pool read survives the tightening: it drops
    /// `owner`, keeps its scope allowlist.
    #[test]
    fn agent_preset_sees_the_shared_pool_not_just_self() {
        let agent = presets().into_iter().find(|r| r.name == "agent").unwrap();
        let gate = effective_filter("ana", &[], std::slice::from_ref(&agent));
        assert_eq!(
            gate.owner_in, None,
            "the agent reads the shared pool — that is the role's stated job"
        );
        assert_eq!(
            gate.access_scopes,
            Some(vec![
                "private".to_string(),
                "domain".to_string(),
                "team".to_string()
            ]),
            "shared_pool drops the OWNER half only; the scope allowlist survives"
        );
    }

    #[test]
    fn owner_filter_reports_sees_only_managed_rows() {
        let sup = presets()
            .into_iter()
            .find(|r| r.name == "supervisor")
            .unwrap();
        let gate = effective_filter(
            "bob",
            &manages(&["ana", "chris"]),
            std::slice::from_ref(&sup),
        );
        assert_eq!(
            gate.owner_in,
            Some(vec!["ana".to_string(), "chris".to_string()])
        );
    }

    #[test]
    fn owner_filter_reports_with_no_manages_reads_nothing() {
        let sup = presets()
            .into_iter()
            .find(|r| r.name == "supervisor")
            .unwrap();
        let gate = effective_filter("bob", &[], std::slice::from_ref(&sup));
        // SOME(EMPTY) IS THE DENY. The old assertion was `None` and called it
        // "reads nothing" — but `None` is "no owner predicate", so the pin
        // asserted the bypass it was written to prevent (audit N1).
        assert_eq!(
            gate.owner_in,
            Some(Vec::new()),
            "no manages claim → empty allowlist (deny), never None"
        );
    }

    #[test]
    fn owner_filter_all_without_shared_pool_narrows_to_self() {
        // A custom role created through POST /roles may say `all` without the
        // capability. It must narrow like `self`, not erase the fence.
        let custom = Role {
            name: "custom-all".into(),
            description: None,
            scopes: vec!["private".into(), "domain".into(), "team".into()],
            owner_filter: "all".into(),
            can: vec!["read".into()],
            panels_default: None,
            panels_hidden: None,
            tools_allowed: None,
        };
        let gate = effective_filter("bob", &[], std::slice::from_ref(&custom));
        assert_eq!(
            gate.owner_in,
            Some(vec!["bob".to_string()]),
            "`all` without shared_pool narrows to the caller"
        );
    }

    #[test]
    fn unknown_owner_filter_narrows_to_self() {
        let mut custom = presets()
            .into_iter()
            .find(|r| r.name == "qa-specialist")
            .unwrap();
        custom.owner_filter = "everyone".into();
        let gate = effective_filter("bob", &[], std::slice::from_ref(&custom));
        assert_eq!(
            gate.owner_in,
            Some(vec!["bob".to_string()]),
            "an unrecognised posture fails closed to self"
        );
    }

    #[test]
    fn admin_sees_all() {
        let admin = presets().into_iter().find(|r| r.name == "admin").unwrap();
        let gate = effective_filter("root", &[], std::slice::from_ref(&admin));
        assert_eq!(gate.access_scopes, None, "admin unrestricted scopes");
        assert_eq!(gate.owner_in, None, "admin unrestricted owner");
    }

    #[test]
    fn validate_rejects_bad_vocab() {
        let mut r = presets().into_iter().next().unwrap();
        r.scopes = vec!["public".to_string()];
        assert!(validate(&r).is_err());
        r = presets().into_iter().next().unwrap();
        r.owner_filter = "everyone".to_string();
        assert!(validate(&r).is_err());
        r = presets().into_iter().next().unwrap();
        r.can = vec!["sudo".to_string()];
        assert!(validate(&r).is_err());
        r = presets().into_iter().next().unwrap();
        r.name = "Bad Name".to_string();
        assert!(validate(&r).is_err());
    }

    #[test]
    fn roles_table_round_trips_presets() {
        let conn = db();
        for r in presets() {
            upsert(&conn, &r).unwrap();
        }
        let loaded = list(&conn).unwrap();
        assert_eq!(loaded.len(), PRESETS_RAW.len());
        let l = load(&conn, "dpo").unwrap().unwrap();
        assert!(l.can("dsar_export") && !l.can("purge"));
        assert!(presets().iter().any(|r| r.name == "dpo" && *r == l));
        // unknown name → None (never synthesized)
        assert!(load(&conn, "nope").unwrap().is_none());
    }

    #[test]
    fn can_tool_matches_wildcard() {
        let admin = presets().into_iter().find(|r| r.name == "admin").unwrap();
        assert!(admin.can_tool("ump.recall"), "admin * allows any tool");
        let agent = presets().into_iter().find(|r| r.name == "agent").unwrap();
        assert!(agent.can_tool("ump.recall"));
        assert!(!agent.can_tool("ump.remember"));
    }
}
