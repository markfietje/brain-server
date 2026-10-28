//! `brain plugin uninstall` — the REGISTRATION act, and the erasure
//! boundary. The other half of the pack lifecycle (plan 28 §4's re-install
//! law completed): the installer imported content through doors that audit;
//! the uninstaller removes the REGISTRATION and leaves the imported DATA
//! exactly where it is.
//!
//! The law, in order:
//!
//! 1. **UNINSTALL IS A REGISTRATION ACT, NEVER AN ERASURE ACT.** The pack
//!    leaves the upgrade path (the register row is MARKED, never deleted —
//!    history is kept); the procedures and rules rows it imported remain,
//!    still governed by the ordinary retention/hold/DSAR machinery.
//!    Bulk-deleting content on uninstall would cross the erasure law
//!    (content may be entangled with live decisions' audit hashes); the
//!    operator erases data through the doors that exist (`brain
//!    source-delete`, `brain shred`) when they choose.
//! 2. **REFUSE-WHILE-LIVE, fail-closed** ([`installed_pack_is_live`]): the
//!    act is refused while anything the pack imported is still live — a
//!    roster principal granted a role the pack upserted, a profile the
//!    pack bound still bound to its domain. The refusal NAMES what is live
//!    and the door that clears it (`brain principal revoke`, the profile
//!    unbind). The C2 installer's transaction map (`install.rs`'s module
//!    doc: profile → roles → procedures → rules) is the checklist reversed.
//! 3. **HISTORY IS KEPT, NOT DELETED.** The register row gains `removed_at`
//!    (additive, presence-guarded, the A-1 replay pattern); after
//!    uninstall the re-install law resets — the SAME version may be
//!    installed again, because the C2 refusal guarded a tampered RE-OFFER
//!    riding a LIVE registration, and with the registration removed a
//!    fresh install is a fresh decision (its bytes are hash-checked
//!    against the signature anyway).
//! 4. **THE DRY-RUN LAW IS THE INSTALLER'S, INHERITED.** Nothing writes
//!    without `--yes`; one `Plugin` audit row lands on the real act. NO
//!    HTTP route — the mint refusal's posture, third time.

use rusqlite::Connection;

/// One live roster principal standing on a role the pack upserted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivePrincipal {
    pub name: String,
    pub role: String,
}

/// One live profile binding on a domain the pack's import mounted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveBinding {
    pub profile: String,
    pub domain: String,
}

/// The live state a REFUSE-WHILE-LIVE refusal names, each item beside the
/// door that clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveState {
    pub principals: Vec<LivePrincipal>,
    pub bindings: Vec<LiveBinding>,
}

impl LiveState {
    pub fn is_live(&self) -> bool {
        !self.principals.is_empty() || !self.bindings.is_empty()
    }

    /// The refusal body: every live thing, named, beside its clearing door.
    pub fn refusal(&self, pack_id: &str) -> String {
        let mut lines = vec![format!(
            "uninstall refused: pack '{pack_id}' is LIVE — the register law refuses \
             while anything the pack imported is still live:"
        )];
        for p in &self.principals {
            lines.push(format!(
                "  - principal '{}' holds role '{}' (upserted by this pack) — \
                 clear it: `brain principal revoke --name {}`",
                p.name, p.role, p.name
            ));
        }
        for b in &self.bindings {
            lines.push(format!(
                "  - profile '{}' is still bound to pack-mounted domain '{}' — \
                 clear it: the profile unbind (the domain's profile write with no profile)",
                b.profile, b.domain
            ));
        }
        lines.push(
            "clear what is named, then uninstall again. The imported DATA stays \
             either way; erasing it is the doors' work (`brain source-delete`, \
             `brain shred`), never the uninstall act's."
                .to_string(),
        );
        lines.join("\n")
    }
}

/// The live-state check: TRUE while anything the pack imported is still
/// live. Two arms, the C2 installer's transaction map reversed:
///
/// * **roles × roster** — the roles store has no provenance column, so the
///   chain IS the provenance: the installer wrote every role act as
///   `kind='reconcile'`, `actor='plugin:<pack-id>'`,
///   `target_hash=hash("role:<name>")`. The join is computed: each role
///   name still in the store is probed against the pack's attributed
///   targets, and a NON-REVOKED roster principal holding such a role is
///   live (the kill-switch table wins — a revoked principal is dead, which
///   is exactly the door the refusal names).
/// * **the profile** — binds are never audited and the pack id rides a
///   hashed detail, so the binding is attributed by its DOMAIN: the
///   installer's domain act went through the one mount door
///   (`kind='reconcile'`, `actor='operator'`, target `domain:<name>`), and
///   a profile still bound to a pack-mounted domain keeps the pack live.
///   Fail-closed by construction: in a multi-pack deployment this can name
///   a binding another pack's import mounted — the named state is real
///   either way, the door clears it, and the register row records the
///   boundary. Packs that declare no domain arm skip this check entirely.
pub fn installed_pack_is_live(conn: &Connection, pack_id: &str) -> Result<LiveState, String> {
    let kinds: Option<String> = conn
        .query_row(
            "SELECT kinds FROM installed_packs WHERE pack_id = ?1 AND removed_at IS NULL",
            [pack_id],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other.to_string()),
        })
        .map_err(|e| format!("installed_packs read: {e} (has the migration run?)"))?;
    let Some(kinds) = kinds else {
        return Err(format!(
            "no live registration for pack '{pack_id}' — `brain plugin list` is the register"
        ));
    };
    let mut live = LiveState::default();

    // ARM 1 — the roles × roster join, computed through the chain.
    let pack_actor = format!("plugin:{pack_id}");
    let role_names: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM roles")
            .map_err(|e| format!("roles read: {e}"))?;
        let rows = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    let mut probe = conn
        .prepare(
            "SELECT 1 FROM audit_events
             WHERE kind = 'reconcile' AND actor = ?1 AND target_hash = ?2 LIMIT 1",
        )
        .map_err(|e| format!("audit read: {e}"))?;
    for name in role_names {
        let target = crate::audit::hash(&format!("role:{name}"));
        let upserted_by_pack = probe
            .query_row(rusqlite::params![pack_actor, target], |r| {
                r.get::<_, i64>(0)
            })
            .is_ok();
        if !upserted_by_pack {
            continue;
        }
        let mut holders = conn
            .prepare(
                "SELECT name FROM principals WHERE role = ?1
                 AND NOT EXISTS(SELECT 1 FROM revoked_principals r WHERE r.principal = name)",
            )
            .map_err(|e| format!("principals read: {e}"))?;
        let rows = holders
            .query_map([&name], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for holder in rows {
            live.principals.push(LivePrincipal {
                name: holder.map_err(|e| e.to_string())?,
                role: name.clone(),
            });
        }
    }

    // ARM 2 — the profile, attributed by its domain's mount.
    if kinds.split(',').any(|k| k.trim() == "domain") {
        let candidates: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT domain, profile FROM domain_profiles")
                .map_err(|e| format!("domain_profiles read: {e}"))?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        for (domain, profile) in candidates {
            let target = crate::audit::hash(&format!("domain:{domain}"));
            let pack_mounted = probe
                .query_row(rusqlite::params!["operator", target], |r| {
                    r.get::<_, i64>(0)
                })
                .is_ok();
            if pack_mounted {
                live.bindings.push(LiveBinding { profile, domain });
            }
        }
    }
    Ok(live)
}

/// The uninstall receipt: the marked register row and the audit row that
/// witnesses the act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallReceipt {
    pub register_id: i64,
    pub version: String,
    pub audit_id: Option<i64>,
}

/// The REGISTRATION act: mark `removed_at`, write ONE `Plugin` audit row,
/// and change NOTHING else — the imported procedures and rules rows stay
/// exactly where they are (the erasure boundary; the ordinary doors erase
/// data when the operator chooses). Fail-closed in order: an unknown
/// pack-id refuses, an already-removed registration refuses (history, not
/// a live registration), a LIVE pack refuses naming what is live and the
/// doors that clear it. `now` is the caller's clock (unix seconds) so the
/// pins drive time.
pub fn uninstall_pack(
    conn: &mut Connection,
    pack_id: &str,
    now: i64,
) -> Result<UninstallReceipt, String> {
    let (register_id, version): (i64, String) = conn
        .query_row(
            "SELECT id, version FROM installed_packs WHERE pack_id = ?1",
            [pack_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| {
            format!("no installed pack named '{pack_id}' — `brain plugin list` is the register")
        })?;
    let removed_at: Option<i64> = conn
        .query_row(
            "SELECT removed_at FROM installed_packs WHERE id = ?1",
            [register_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("installed_packs read: {e}"))?;
    if let Some(at) = removed_at {
        return Err(format!(
            "pack '{pack_id}' was already removed at unix {at} — the register row is \
             HISTORY, not a live registration; installing again is a fresh decision"
        ));
    }
    let live = installed_pack_is_live(conn, pack_id)?;
    if live.is_live() {
        return Err(live.refusal(pack_id));
    }

    // BEGIN IMMEDIATE — the installer's own write discipline for a
    // read-then-mark transition. One tx: the mark and its audit row commit
    // together or not at all.
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| format!("tx: {e}"))?;
    let marked = tx
        .execute(
            "UPDATE installed_packs SET removed_at = ?2
             WHERE id = ?1 AND removed_at IS NULL",
            rusqlite::params![register_id, now],
        )
        .map_err(|e| format!("register mark: {e}"))?;
    if marked != 1 {
        return Err(format!(
            "register mark: pack '{pack_id}' moved under the act — nothing written"
        ));
    }
    // ONE audit row, the installer's own kind: the chain's evidence the
    // registration ENDED. The detail names the erasure boundary, because a
    // chain reader in five years deserves the law in the row, not in a doc.
    let audit_id = crate::audit::record(
        &tx,
        crate::audit::AuditKind::Plugin,
        "operator",
        &format!("pack:{pack_id}"),
        crate::audit::AuditStatus::Ok,
        &format!(
            "uninstall {pack_id}@{version} register row {register_id} marked removed — \
             the imported DATA stays; erasure is the doors' work (brain source-delete, \
             brain shred), never the uninstall act's"
        ),
    );
    tx.commit().map_err(|e| format!("tx commit: {e}"))?;
    Ok(UninstallReceipt {
        register_id,
        version,
        audit_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::install;

    fn db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("db");
        crate::migration::run_migration(&mut conn, 512).expect("migrate fixture");
        conn
    }

    /// A minimal VALID pack's entries: domain (+optional profile), optional
    /// roles, one rule. `kinds` always names what the entries carry.
    fn pack_entries(profile: Option<&str>, with_roles: bool) -> Vec<(&'static str, Vec<u8>)> {
        let mut kinds = vec!["domain", "rules"];
        if with_roles {
            kinds.insert(1, "roles");
        }
        let mut entries = vec![
            (
                "manifest.json",
                serde_json::json!({
                    "id": "support-pack", "version": "0.1.0",
                    "engine": ">=1.0.0", "kinds": kinds
                })
                .to_string()
                .into_bytes(),
            ),
            (
                "domain.json",
                match profile {
                    Some(p) => serde_json::json!({
                        "name": "support", "label": "Support desk",
                        "retention_days": 90, "profile": p
                    }),
                    None => serde_json::json!({
                        "name": "support", "label": "Support desk", "retention_days": 90
                    }),
                }
                .to_string()
                .into_bytes(),
            ),
            (
                "rules.json",
                serde_json::json!([{
                    "jurisdiction": "ph", "subject": "support-tickets",
                    "rule_key": "ticket_retention",
                    "body": "Resolved ticket corpus is retained 90 days post-resolution.",
                    "source_ref": "pack#retention",
                    "effective_at": 1_700_000_000, "revision": 1
                }])
                .to_string()
                .into_bytes(),
            ),
        ];
        if with_roles {
            entries.push((
                "roles.json",
                serde_json::json!([{
                    "name": "support-supervisor",
                    "scopes": ["domain"], "owner_filter": "reports",
                    "can": ["read", "write", "approve"]
                }])
                .to_string()
                .into_bytes(),
            ));
        }
        entries
    }

    fn install_fixture(conn: &mut Connection, profile: Option<&str>, with_roles: bool) {
        let entries = pack_entries(profile, with_roles);
        let refs: Vec<(&str, Vec<u8>)> = entries.iter().map(|(k, v)| (*k, v.clone())).collect();
        let pack = crate::packs::validate_pack(&refs).expect("the fixture pack validates");
        install::apply(conn, &pack, "a", "m", entries.len(), 1_000)
            .expect("the fixture pack installs");
    }

    /// U-2 pin 1 — the clean path: a fresh install with no principals and
    /// no profile binding reads NOT live. Uninstall may proceed.
    #[test]
    fn a_fresh_install_with_no_live_state_reads_clean() {
        let mut conn = db();
        install_fixture(&mut conn, None, false);
        let state = installed_pack_is_live(&conn, "support-pack").expect("the register reads");
        assert!(
            !state.is_live(),
            "no principals, no bindings: the pack is not live: {state:?}"
        );
    }
    /// U-2 pin 2 — the refuse-while-live arm, ROLES: a roster principal
    /// granted a role the pack upserted is LIVE, and the refusal NAMES the
    /// principal, the role, and the door that clears it.
    #[test]
    fn a_pack_with_a_live_principal_refuses_and_names_it() {
        let mut conn = db();
        install_fixture(&mut conn, None, true);
        crate::auth::principals::grant(
            &conn,
            "support-agent",
            "support-supervisor",
            "support",
            &[],
            &[],
            &crate::auth::principals::hash_token("bearer"),
            2_000,
            None,
        )
        .expect("the grant lands");
        let state = installed_pack_is_live(&conn, "support-pack").expect("the register reads");
        assert_eq!(
            state.principals,
            vec![LivePrincipal {
                name: "support-agent".into(),
                role: "support-supervisor".into(),
            }],
            "the live principal is named with the role that holds them"
        );
        let refusal = state.refusal("support-pack");
        assert!(refusal.contains("support-agent"), "{refusal}");
        assert!(refusal.contains("support-supervisor"), "{refusal}");
        assert!(
            refusal.contains("brain principal revoke --name support-agent"),
            "the refusal names the clearing door: {refusal}"
        );
    }

    /// U-2 pin 3 — the door works: after `brain principal revoke` (this is
    /// the exact function the CLI door calls), the pack reads clean and
    /// uninstall may proceed.
    #[test]
    fn after_the_revoke_door_the_pack_reads_clean() {
        let mut conn = db();
        install_fixture(&mut conn, None, true);
        crate::auth::principals::grant(
            &conn,
            "support-agent",
            "support-supervisor",
            "support",
            &[],
            &[],
            &crate::auth::principals::hash_token("bearer"),
            2_000,
            None,
        )
        .expect("the grant lands");
        assert!(
            installed_pack_is_live(&conn, "support-pack")
                .expect("reads")
                .is_live()
        );
        crate::auth::principals::revoke_grant(
            &conn,
            "support-agent",
            "operator revoke",
            "operator",
            3_000,
        )
        .expect("the revoke lands");
        let state = installed_pack_is_live(&conn, "support-pack").expect("the register reads");
        assert!(
            !state.is_live(),
            "the kill-switch wins: a revoked principal is not live: {state:?}"
        );
    }

    /// U-2 pin 4 — the refuse-while-live arm, PROFILE: a profile the pack
    /// bound is still bound to its (pack-mounted) domain, the state names
    /// both, and the unbind door — `profile::bind(domain, None)`, the exact
    /// function the HTTP door and the installer share — clears it.
    #[test]
    fn the_profile_arm_refuses_while_the_binding_stands_and_clears_at_the_unbind_door() {
        let mut conn = db();
        install_fixture(&mut conn, Some("call-center"), false);
        let state = installed_pack_is_live(&conn, "support-pack").expect("the register reads");
        assert_eq!(
            state.bindings,
            vec![LiveBinding {
                profile: "call-center".into(),
                domain: "support".into(),
            }],
            "the live binding is named with its domain"
        );
        let refusal = state.refusal("support-pack");
        assert!(refusal.contains("call-center"), "{refusal}");
        assert!(refusal.contains("support"), "{refusal}");
        assert!(
            refusal.contains("unbind"),
            "the refusal names the clearing door: {refusal}"
        );
        crate::profile::bind(&conn, "support", None).expect("the unbind lands");
        let state = installed_pack_is_live(&conn, "support-pack").expect("the register reads");
        assert!(!state.is_live(), "the unbind clears the arm: {state:?}");
    }

    /// U-3 — THE DATA LAW, COUNT-PINNED: the act marks the register and
    /// writes ONE audit row, and changes NOTHING else. Every content count
    /// (rules, roles, the domain's knowledge) is measured before and after
    /// and must be equal; the governance-visibility check reads the rule
    /// back through the rules door itself — the imported rows are not just
    /// present, they are still governed by the machinery that always
    /// governed them.
    #[test]
    fn the_act_marks_the_row_writes_one_audit_row_and_moves_nothing_else() {
        let mut conn = db();
        install_fixture(&mut conn, None, true);
        let count = |conn: &Connection, sql: &str| -> i64 {
            conn.query_row(sql, [], |r| r.get(0)).expect("count")
        };
        let before = (
            count(&conn, "SELECT COUNT(*) FROM rules"),
            count(
                &conn,
                "SELECT COUNT(*) FROM roles WHERE name = 'support-supervisor'",
            ),
            count(
                &conn,
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'plugin'",
            ),
        );
        assert_eq!(before.1, 1, "the fixture's role row is present pre-act");
        let receipt = uninstall_pack(&mut conn, "support-pack", 5_000).expect("the act lands");
        assert_eq!(receipt.version, "0.1.0");
        assert!(receipt.audit_id.is_some(), "the act is witnessed");
        let after = (
            count(&conn, "SELECT COUNT(*) FROM rules"),
            count(
                &conn,
                "SELECT COUNT(*) FROM roles WHERE name = 'support-supervisor'",
            ),
            count(
                &conn,
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'plugin'",
            ),
        );
        assert_eq!(before.0, after.0, "zero rule rows change — the data law");
        assert_eq!(before.1, after.1, "zero role rows change — the data law");
        assert_eq!(before.2 + 1, after.2, "exactly ONE audit row lands");
        let (version, removed_at): (String, Option<i64>) = conn
            .query_row(
                "SELECT version, removed_at FROM installed_packs WHERE pack_id = 'support-pack'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(version, "0.1.0", "the row is marked, never rewritten");
        assert_eq!(removed_at, Some(5_000), "the mark is the caller's clock");
        // GOVERNANCE VISIBILITY: the rule reads back through the ordinary
        // door, not through a side channel.
        let stored = crate::service::rules::list(&conn).expect("the rules door reads");
        assert!(
            stored.iter().any(|r| r.rule_key == "ticket_retention"),
            "the imported rule is still listed by the rules door: {:?}",
            stored
                .iter()
                .map(|r| r.rule_key.as_str())
                .collect::<Vec<_>>()
        );
    }

    /// U-3 — the fail-closed triage: an unknown pack-id refuses; a LIVE
    /// pack refuses with the naming refusal; an already-removed
    /// registration refuses (history, not a live registration).
    #[test]
    fn the_act_refuses_unknown_live_and_already_removed() {
        let mut conn = db();
        let err = uninstall_pack(&mut conn, "no-such-pack", 1_000).unwrap_err();
        assert!(err.contains("no installed pack named"), "{err}");

        install_fixture(&mut conn, None, true);
        crate::auth::principals::grant(
            &conn,
            "support-agent",
            "support-supervisor",
            "support",
            &[],
            &[],
            &crate::auth::principals::hash_token("bearer"),
            2_000,
            None,
        )
        .expect("the grant lands");
        let err = uninstall_pack(&mut conn, "support-pack", 3_000).unwrap_err();
        assert!(err.contains("uninstall refused"), "{err}");
        assert!(err.contains("brain principal revoke"), "{err}");
        // NOTHING moved on the refusal — the register is untouched.
        let removed_at: Option<i64> = conn
            .query_row(
                "SELECT removed_at FROM installed_packs WHERE pack_id = 'support-pack'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(removed_at, None, "a refusal writes nothing");

        crate::auth::principals::revoke_grant(
            &conn,
            "support-agent",
            "operator revoke",
            "operator",
            4_000,
        )
        .expect("the revoke lands");
        uninstall_pack(&mut conn, "support-pack", 5_000).expect("the act lands");
        let err = uninstall_pack(&mut conn, "support-pack", 6_000).unwrap_err();
        assert!(err.contains("already removed"), "{err}");
        assert!(err.contains("fresh decision"), "{err}");
    }

    /// U-1 — the A-1 replay pattern (the `principals.manages` precedent):
    /// a DB that already carries a register row gains the column without
    /// losing the row, and re-running the migration changes nothing (the
    /// presence-guarded ALTER is idempotent — the column lands exactly
    /// once). This is what the migrate-rehearse copy does on every
    /// rehearsal: VACUUM INTO carries the column, `run_migration` lands it
    /// on DBs that predate it.
    #[test]
    fn the_removed_at_migration_is_replay_safe_and_keeps_rows() {
        let mut conn = db();
        conn.execute(
            "INSERT INTO installed_packs(pack_id, version, kinds, manifest_hash,
                 archive_hash, file_count, import_audit_id, imported_at)
             VALUES ('support-pack', '0.1.0', 'domain,rules', 'm', 'a', 3, NULL, 1_000)",
            [],
        )
        .expect("the C2-shaped register row");
        // THE REPLAY: the same DB migrated again — every boot, and the
        // rehearse tool's copy phase.
        crate::migration::run_migration(&mut conn, 512).expect("a second migration run");
        crate::migration::run_migration(&mut conn, 512).expect("a third migration run");
        let columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('installed_packs')
                 WHERE name = 'removed_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(columns, 1, "the column lands exactly once");
        let (version, removed_at): (String, Option<i64>) = conn
            .query_row(
                "SELECT version, removed_at FROM installed_packs WHERE pack_id = 'support-pack'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("the row survived the replay");
        assert_eq!(version, "0.1.0", "a rehearsal forgets no register row");
        assert_eq!(
            removed_at, None,
            "NULL = a live registration: every pre-uninstall row reads live"
        );
    }
}
