//! The kernel-side plugin principal roster (M3-g, plan 27): GRANT, REVOKE,
//! LIST — operator acts through the `brain principal` CLI, never HTTP (an
//! HTTP mint is a privilege-escalation surface with zero measured need;
//! minting happens on the machine that owns the token files, the same
//! posture as `install-service.sh`) — and the auth middleware's plugin lane,
//! which resolves a minted bearer against this roster per request.
//!
//! The law (plan 22 as amended by M3-f's R6): an extension is a kernel
//! CLIENT — its own credential, a scoped role; row-level authz is the
//! sandbox; nothing extension-authored ever runs inside the kernel. The
//! roster carries DATA grants only — name, role, domain, tools. A token is
//! born here (64 hex chars of random), lands in a 0600 file the operator
//! hands the plugin, and is stored NOWHERE in raw form: the roster holds its
//! SHA-256, and the bearer the middleware sees is hashed the same way before
//! the lookup, so a database leak mints no credentials.
//!
//! The principal the lane builds is deliberately the existing authz
//! machinery, not a new one: scopes = write over its ONE domain (write
//! implies read down, so the preset's `can: ["read","write"]` is exactly a
//! `write:*/<domain>` scope); roles = the granted role name, resolved
//! against the roles store by the SAME `authorize_tool`/`authorize_role`
//! seams every other principal passes (`plugin` ships `tools_allowed: []` —
//! the day-zero floor — and the roster's `tools` column is the only
//! widening, enforced by the roster arm in `authorize_tool`, consulted only
//! for [`PrincipalKind::Plugin`] so a JWT `sub` colliding with a roster
//! name inherits nothing).

use rusqlite::{Connection, OptionalExtension, params};

use super::policy::{Action, Principal, PrincipalKind, Scope};

/// One roster row: the grant as the operator wrote it, plus the kill-switch
/// state the middleware will honor (`revoked` names live in
/// `revoked_principals`, not here — one kill-switch table, shared with the
/// mesh and the agent lane).
#[derive(Debug, Clone, PartialEq)]
pub struct RosterRow {
    pub name: String,
    pub role: String,
    pub domain: String,
    pub tools: Vec<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub revoked: bool,
}

/// Mint one bearer: 64 hex chars from 32 random bytes (the agent-token
/// shape; `openssl rand -hex 32` in `install-service.sh` is the shell
/// spelling of the same length and entropy).
pub fn mint_token() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 hex of a presented bearer — the ONLY token material the roster
/// stores or compares.
pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    hex::encode(h.finalize())
}

/// Validate a grant against the SAME vocabularies the rest of the tree
/// enforces: the name is domain-charset (it becomes an audit actor, a
/// kill-switch key and a filename), the role must EXIST in the roles store
/// (a grant against an unknown role is a typo wearing an escalation's
/// clothes), the domain is domain-charset, and each named tool fits the
/// role-store tool bound. `plugin` is not the only grantable role — the
/// operator names the posture — but every grant names an EXISTING one.
pub fn validate_grant(
    conn: &Connection,
    name: &str,
    role: &str,
    domain: &str,
    tools: &[&str],
) -> Result<(), String> {
    if !crate::storage_layout::is_valid_domain(name) {
        return Err(format!(
            "invalid principal name '{name}' (lowercase alnum + hyphen, max 63 — it \
             becomes an audit actor and a kill-switch key)"
        ));
    }
    if crate::role::load(conn, role)?.is_none() {
        return Err(format!(
            "role '{role}' is not in the roles store — a grant names an existing \
             role (ship-with or operator-created), never an invention"
        ));
    }
    if !crate::storage_layout::is_valid_domain(domain) {
        return Err(format!(
            "invalid domain '{domain}' (lowercase alnum + hyphen, max 63)"
        ));
    }
    for t in tools {
        if t.is_empty() || t.len() > 63 {
            return Err(format!("tool name '{t}' invalid (1..=63 chars)"));
        }
    }
    Ok(())
}

/// Grant (or deliberately re-grant) one principal: the roster row upserted,
/// the audit row written. The token itself NEVER passes through here — the
/// caller mints it, hashes it, and writes the raw form only to the 0600
/// file. A re-grant under the SAME name rotates the hash and the fields
/// (the roster is identity, the audit chain is history) — but a REVOKED
/// name stays dead: the kill-switch table wins at request time, so the
/// remedy for a revoked principal is a new name, never an un-revoke.
pub fn grant(
    conn: &Connection,
    name: &str,
    role: &str,
    domain: &str,
    tools: &[&str],
    token_hash: &str,
    now: i64,
    expires_at: Option<i64>,
) -> Result<(), String> {
    validate_grant(conn, name, role, domain, tools)?;
    conn.execute(
        "INSERT INTO principals(name, role, domain, tools, token_hash, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(name) DO UPDATE SET
             role = excluded.role,
             domain = excluded.domain,
             tools = excluded.tools,
             token_hash = excluded.token_hash,
             created_at = excluded.created_at,
             expires_at = excluded.expires_at",
        params![
            name,
            role,
            domain,
            tools.join(","),
            token_hash,
            now,
            expires_at
        ],
    )
    .map_err(|e| format!("principals table: {e} (has the migration run?)"))?;
    crate::audit::record(
        conn,
        crate::audit::AuditKind::Auth,
        "operator",
        &format!("principal:{name}"),
        crate::audit::AuditStatus::Ok,
        &format!(
            "grant: role={role} domain={domain} tools={}",
            tools.join(",")
        ),
    );
    Ok(())
}

/// Revoke one principal: the kill-switch row (same upsert shape the mesh's
/// revoke writes — latest revocation wins) + the audit row. The CLI act
/// writes row + audit; the HTTP route (`POST /ops/agents/revoke`) remains
/// the fuller kill-switch for mesh identities (it drains in-flight runs).
/// The live consequence is the middleware's next `is_revoked` read: the
/// minted token 401s `identity_revoked` on the next call.
pub fn revoke_grant(
    conn: &Connection,
    name: &str,
    reason: &str,
    revoked_by: &str,
    now: i64,
) -> Result<(), String> {
    if name.is_empty() {
        return Err("principal name required".to_string());
    }
    conn.execute(
        "INSERT INTO revoked_principals(principal, revoked_at, reason, revoked_by)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(principal) DO UPDATE SET
             revoked_at = excluded.revoked_at,
             reason = excluded.reason,
             revoked_by = excluded.revoked_by",
        params![name, now, reason, revoked_by],
    )
    .map_err(|e| format!("revoked_principals: {e} (has the migration run?)"))?;
    crate::audit::record(
        conn,
        crate::audit::AuditKind::Auth,
        revoked_by,
        &format!("principal:{name}"),
        crate::audit::AuditStatus::Ok,
        &format!("revoke:{reason}"),
    );
    Ok(())
}

/// The roster, oldest first, each row carrying its kill-switch state.
pub fn list(conn: &Connection) -> Result<Vec<RosterRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT p.name, p.role, p.domain, p.tools, p.created_at, p.expires_at,
                    EXISTS(SELECT 1 FROM revoked_principals r WHERE r.principal = p.name)
             FROM principals p ORDER BY p.created_at, p.name",
        )
        .map_err(|e| format!("principals table: {e} (has the migration run?)"))?;
    let rows = stmt
        .query_map([], |r| {
            let tools_csv: String = r.get(3)?;
            Ok(RosterRow {
                name: r.get(0)?,
                role: r.get(1)?,
                domain: r.get(2)?,
                tools: tools_csv
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect(),
                created_at: r.get(4)?,
                expires_at: r.get(5)?,
                revoked: r.get::<_, i64>(6)? != 0,
            })
        })
        .map_err(|e| format!("principals table: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("principals read: {e}"))
}

/// The middleware's plugin lane: resolve a presented bearer against the
/// roster, fail-closed on every leg.
///
/// * `Ok(None)` — not a roster bearer; the middleware falls through to its
///   deny arm unchanged.
/// * `Err(code)` — a roster bearer that fails a LIVING check: `identity_
///   expired` past `expires_at`, `identity_revoked` on the kill-switch.
///   Both are 401s that name their cause.
/// * `Ok(Some(principal))` — the grant, as the existing authz machinery
///   sees it (see the module doc): one domain's write scope, the granted
///   role, the `Plugin` kind.
///
/// `now` is the caller's clock (unix seconds) so the lane is pure input →
/// decision and the tests drive expiry without sleeping.
pub fn plugin_principal_for_token(
    conn: &Connection,
    presented: &str,
    now: i64,
) -> Result<Option<Principal>, String> {
    let token_hash = hash_token(presented);
    let row = conn
        .query_row(
            "SELECT name, role, domain, expires_at FROM principals WHERE token_hash = ?1",
            params![token_hash],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("principal store error: {e}"))?;
    let Some((name, role, domain, expires_at)) = row else {
        return Ok(None);
    };
    if expires_at.is_some_and(|exp| exp < now) {
        return Err("identity_expired".to_string());
    }
    if super::super::workflow::mesh::is_revoked(conn, &name)
        .map_err(|e| format!("revocation store error: {e}"))?
    {
        return Err("identity_revoked".to_string());
    }
    let domain_lc = domain.trim().to_ascii_lowercase();
    Ok(Some(Principal {
        sub: name,
        tenant: crate::audit::DEFAULT_TENANT.to_string(),
        scopes: vec![Scope {
            action: Action::Write,
            team: "*".to_string(),
            domain: domain_lc,
        }],
        jti: "plugin".to_string(),
        roles: vec![role],
        manages: vec![],
        kind: PrincipalKind::Plugin,
    }))
}

/// The per-principal tool grant: the named set from the roster row this
/// principal's `sub` resolves to, or an empty vec. Called ONLY with
/// `kind == Plugin` (the caller guards it) — the kind is what keeps a
/// colliding JWT `sub` from inheriting a plugin's grants.
pub fn named_tools(conn: &Connection, name: &str) -> Result<Vec<String>, String> {
    let csv: Option<String> = conn
        .query_row(
            "SELECT tools FROM principals WHERE name = ?1",
            params![name],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("principal store error: {e}"))?;
    Ok(csv
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("db");
        crate::migration::run_migration(&mut conn, 0).expect("migrate fixture");
        conn
    }

    fn grant_fixture(conn: &Connection, token: &str) -> RosterRow {
        grant(
            conn,
            "edu-plugin",
            "plugin",
            "education",
            &["ump.recall", "ump.get"],
            &hash_token(token),
            1_000,
            None,
        )
        .expect("grant");
        list(conn).expect("list").remove(0)
    }

    #[test]
    fn grant_round_trips_through_the_roster_and_the_audit_chain() {
        let conn = db();
        let row = grant_fixture(&conn, "bearer-one");
        assert_eq!(row.name, "edu-plugin");
        assert_eq!(row.role, "plugin");
        assert_eq!(row.domain, "education");
        assert_eq!(row.tools, vec!["ump.recall", "ump.get"]);
        assert_eq!(row.revoked, false);

        // the roster stores the HASH, never the bearer
        let stored: String = conn
            .query_row(
                "SELECT token_hash FROM principals WHERE name = 'edu-plugin'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, hash_token("bearer-one"));
        assert_ne!(stored, "bearer-one");

        // and the grant left its audit row: kind auth, the actor is the
        // operator, and the target names the principal. Columns carry HASHES
        // (target_hash/detail_hash) — the plaintext act lives only in the
        // chain's input, so the count + actor + kind are what a test can
        // read back.
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='auth' AND actor='operator'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "the grant wrote one audited act");
    }

    #[test]
    fn regrant_rotates_the_hash_and_the_roster_stays_one_row() {
        let conn = db();
        grant_fixture(&conn, "bearer-one");
        grant(
            &conn,
            "edu-plugin",
            "plugin",
            "education",
            &[],
            &hash_token("bearer-two"),
            2_000,
            None,
        )
        .expect("re-grant");
        assert_eq!(list(&conn).unwrap().len(), 1, "the roster is identity");
        // the OLD bearer is dead: no row carries its hash any more
        assert!(
            plugin_principal_for_token(&conn, "bearer-one", 3_000)
                .unwrap()
                .is_none()
        );
        assert!(
            plugin_principal_for_token(&conn, "bearer-two", 3_000)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn the_lane_resolves_the_grant_and_fails_closed_on_every_leg() {
        let conn = db();
        let token = "minted-bearer";
        grant_fixture(&conn, token);

        // the happy lane: the principal is the grant, built from the ROSTER
        let p = plugin_principal_for_token(&conn, token, 2_000)
            .unwrap()
            .expect("a live grant resolves");
        assert_eq!(p.sub, "edu-plugin");
        assert_eq!(p.roles, vec!["plugin".to_string()]);
        assert_eq!(p.kind, PrincipalKind::Plugin);
        assert_eq!(p.scopes.len(), 1);
        assert_eq!(p.scopes[0].domain, "education");
        assert_eq!(p.scopes[0].action, Action::Write);

        // an unknown bearer is not a roster principal — the deny arm's world
        assert!(
            plugin_principal_for_token(&conn, "no-such-bearer", 2_000)
                .unwrap()
                .is_none()
        );

        // the kill-switch leg: revoke, then the SAME bearer 401s by name
        revoke_grant(&conn, "edu-plugin", "operator revoke", "operator", 3_000).unwrap();
        let err = plugin_principal_for_token(&conn, token, 4_000).unwrap_err();
        assert_eq!(err, "identity_revoked", "the kill-switch names itself");

        // the expiry leg: a past expires_at 401s by name, before revocation
        // is even consulted
        let conn2 = db();
        grant(
            &conn2,
            "edu-plugin",
            "plugin",
            "education",
            &[],
            &hash_token("expiring"),
            1_000,
            Some(2_000),
        )
        .unwrap();
        let err = plugin_principal_for_token(&conn2, "expiring", 3_000).unwrap_err();
        assert_eq!(err, "identity_expired", "expiry names itself");
        // ... and the same grant is alive BEFORE the horizon
        assert!(
            plugin_principal_for_token(&conn2, "expiring", 1_500)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_revoked_name_stays_dead_across_a_regrant() {
        let conn = db();
        grant_fixture(&conn, "bearer-one");
        revoke_grant(&conn, "edu-plugin", "compromised", "operator", 2_000).unwrap();
        // re-granting the same name rotates the roster row but does NOT
        // revive it — the kill-switch table wins, so the remedy is a new
        // name, never an un-revoke
        grant(
            &conn,
            "edu-plugin",
            "plugin",
            "education",
            &[],
            &hash_token("bearer-two"),
            3_000,
            None,
        )
        .expect("the upsert lands");
        let err = plugin_principal_for_token(&conn, "bearer-two", 4_000).unwrap_err();
        assert_eq!(err, "identity_revoked", "revive nothing");
        let row = &list(&conn).unwrap()[0];
        assert!(row.revoked, "the roster shows the kill-switch state");
    }

    #[test]
    fn validate_refuses_an_unknown_role_and_bad_names() {
        let conn = db();
        // the role must exist — `plugin` ships, `root` does not
        assert!(validate_grant(&conn, "edu-plugin", "plugin", "education", &[]).is_ok());
        assert!(
            validate_grant(&conn, "edu-plugin", "root", "education", &[])
                .unwrap_err()
                .contains("not in the roles store")
        );
        assert!(validate_grant(&conn, "Bad Name", "plugin", "education", &[]).is_err());
        assert!(validate_grant(&conn, "edu-plugin", "plugin", "no such domain", &[]).is_err());
        assert!(validate_grant(&conn, "edu-plugin", "plugin", "education", &[""]).is_err());
        assert!(validate_grant(&conn, "edu-plugin", "plugin", "education", &["ok-tool"]).is_ok());
    }

    #[test]
    fn named_tools_reads_the_roster_not_the_token() {
        let conn = db();
        grant_fixture(&conn, "bearer-one");
        assert_eq!(
            named_tools(&conn, "edu-plugin").unwrap(),
            vec!["ump.recall".to_string(), "ump.get".to_string()]
        );
        assert!(named_tools(&conn, "no-such-principal").unwrap().is_empty());
    }
}
