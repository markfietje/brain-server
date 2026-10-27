//! `brain plugin install` — the signed chain (the plugin-integration
//! drill's shape, plan 28 §4):
//! **verify → dry-run → import**, each import act through a door that
//! already exists and already audits.
//!
//! The law, in order (C2 READ FIRST ⑤):
//!
//! 1. **verify** the detached minisign signature against the PINNED pack
//!    key ([`super::signature`]) — a pack that fails its signature is
//!    never parsed, let alone imported. THE two-key law: a pack signed
//!    with the UPDATER key is a refusal naming both keynums.
//! 2. **DRY-RUN is the default surface** — every row that WOULD land is
//!    printed, and NOTHING writes without `--yes` (the `brain client add
//!    --yes` convention). `--dry-run` says it explicitly; its absence plus
//!    `--yes`'s absence runs the same dry-run with a hint.
//! 3. **import through the EXISTING layers**, one transaction, domain →
//!    roles → procedures → rules: the profile binding write
//!    ([`crate::profile::bind`], fail-closed on an unknown profile), the
//!    role store's own validate+upsert+audit (the `/roles` route's exact
//!    posture), the procedure door's internals
//!    ([`crate::service::procedure::store_procedure`], which writes its own
//!    `Procedure` audit row inside the tx), and the NEW rules door
//!    ([`crate::service::rules::insert`], one `Rules` audit row per rule).
//!    A pack import that fails any act rolls the WHOLE import back — a
//!    half-mounted pack is not a state this law permits.
//! 4. **one pack import row** in the register (`installed_packs`, the
//!    roster door's neighborhood) naming id + version + content hashes,
//!    plus the `Plugin` audit row that witnesses the import on the chain.
//!
//! **The re-install law (CHSEN, recorded):** re-installing the same
//! id+version is REFUSED — idempotent success is how a tampered re-offer
//! would ride in wearing the previous install's clothes; a HIGHER version
//! of the same id is the upgrade path (the register row updates, the audit
//! chain carries both). No uninstall verb ships: the register row plus the
//! audit trail is the record, and removing a vertical is a round of its
//! own.
//!
//! Per-row acts attribute `plugin:<pack-id>` as the actor (the CONTENT's
//! provenance); the import act itself attributes `operator` (the hand that
//! ran it). The audit chain keeps both.

use rusqlite::Connection;

use super::ValidatedPack;

/// One row the import WOULD land (or did land): the dry-run's printed
/// lines and the import's count come from the SAME plan, so the pin
/// "dry-run's printed rows == the imported rows" is structural.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRow {
    Domain {
        name: String,
        profile: Option<String>,
    },
    Role {
        name: String,
    },
    Procedure {
        title: String,
    },
    Rule {
        natural_key: String,
        revision: i64,
    },
}

/// The import plan: every row `apply` would land, in order.
pub fn plan(pack: &ValidatedPack) -> Vec<PlanRow> {
    let mut rows = Vec::new();
    if let Some(domain) = &pack.domain {
        rows.push(PlanRow::Domain {
            name: domain.name.clone(),
            profile: domain.profile.clone(),
        });
    }
    for role in &pack.roles {
        rows.push(PlanRow::Role {
            name: role.name.clone(),
        });
    }
    for (stem, _) in &pack.procedures {
        rows.push(PlanRow::Procedure {
            title: procedure_title(stem),
        });
    }
    for rule in &pack.rules {
        rows.push(PlanRow::Rule {
            natural_key: format!("{}:{}:{}", rule.jurisdiction, rule.subject, rule.rule_key),
            revision: rule.revision,
        });
    }
    rows
}

/// The line a dry-run prints for one planned row — the exact shape the
/// import's own per-act trail takes.
pub fn plan_line(row: &PlanRow) -> String {
    match row {
        PlanRow::Domain { name, profile } => match profile {
            Some(p) => format!("domain {name} (profile {p})"),
            None => format!("domain {name}"),
        },
        PlanRow::Role { name } => format!("role {name}"),
        PlanRow::Procedure { title } => format!("procedure \"{title}\""),
        PlanRow::Rule {
            natural_key,
            revision,
        } => format!("rule {natural_key} @r{revision}"),
    }
}

fn procedure_title(stem: &str) -> String {
    format!("procedure pack: {stem}")
}

/// The import outcome: what landed, under what register row id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReceipt {
    pub register_id: i64,
    pub rows_landed: usize,
    pub audit_refs: Vec<i64>,
}

/// The import error: which act refused, naming its door.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError {
    pub act: &'static str,
    pub message: String,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "import refused at the {} act: {}",
            self.act, self.message
        )
    }
}

/// The installed-register law: refuse a same-version re-install, admit a
/// higher version. Returns Ok(existing_id) when the register row may
/// update, Err when the offer is already installed verbatim.
fn re_install_law(
    conn: &Connection,
    pack_id: &str,
    version: &str,
) -> Result<Option<i64>, ImportError> {
    let existing: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, version FROM installed_packs WHERE pack_id = ?1",
            [pack_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other.to_string()),
        })
        .map_err(|e| ImportError {
            act: "register",
            message: e,
        })?;
    let offered = super::parse_version_tuple(version);
    match (existing, offered) {
        (Some((_id, installed_version)), _) if installed_version == version => Err(ImportError {
            act: "register",
            message: format!(
                "pack '{pack_id}' version {version} is ALREADY INSTALLED — re-install \
                 is refused (a tampered re-offer must not pass as idempotent success); \
                 a higher version is the upgrade path"
            ),
        }),
        (Some((_, installed_version)), Some(offered))
            if super::parse_version_tuple(&installed_version) > Some(offered) =>
        {
            Err(ImportError {
                act: "register",
                message: format!(
                    "pack '{pack_id}' {version} is OLDER than the installed \
                     {installed_version} — the register moves forward"
                ),
            })
        }
        (existing, _) => Ok(existing.map(|(id, _)| id)),
    }
}

/// Apply the import: one transaction, the acts in law's order, every act
/// audited. `now` is the caller's clock (unix seconds) so the pins drive
/// time. The caller has verified the signature and validated the pack —
/// this function is the door, not the bouncer.
pub fn apply(
    conn: &mut Connection,
    pack: &ValidatedPack,
    archive_hash: &str,
    manifest_hash: &str,
    file_count: usize,
    now: i64,
) -> Result<ImportReceipt, ImportError> {
    let pack_id = pack.manifest.id.clone();
    let version = pack.manifest.version.clone();
    let actor = format!("plugin:{pack_id}");

    // The register law BEFORE any act: a verbatim re-install never touches
    // a row, so the refusal is total.
    let existing_register = re_install_law(conn, &pack_id, &version)?;

    // BEGIN IMMEDIATE — the write-discipline law for read-modify-write
    // transitions (the audit chain's tip read-write and the register's
    // read-then-write both need the write lock from the start; the CLI is
    // the single writer here).
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| ImportError {
            act: "tx",
            message: e.to_string(),
        })?;
    let mut audit_refs = Vec::new();

    // 1 · the domain act — the profile binding through profile::bind (the
    // /domains/{d}/profile write, fail-closed on an unknown profile).
    if let Some(domain) = &pack.domain {
        if let Some(profile) = &domain.profile {
            crate::profile::bind(&tx, &domain.name, Some(profile)).map_err(|e| ImportError {
                act: "domain",
                message: e,
            })?;
        }
        let id = crate::audit::record(
            &tx,
            crate::audit::AuditKind::Reconcile,
            "operator",
            &format!("domain:{}", domain.name),
            crate::audit::AuditStatus::Ok,
            // the create path's own detail, extended with the pack's mark
            &format!(
                "domain mounted by pack {pack_id}@{version} (label \"{}\", retention {}d)",
                domain.label, domain.retention_days
            ),
        );
        if let Some(id) = id {
            audit_refs.push(id);
        }
    }

    // 2 · the roles act — the role store's own validate+upsert+audit, the
    // /roles route's exact posture (kind Reconcile, target "role:<name>").
    for role in &pack.roles {
        crate::role::validate(role).map_err(|e| ImportError {
            act: "roles",
            message: e,
        })?;
        crate::role::upsert(&tx, role).map_err(|e| ImportError {
            act: "roles",
            message: e,
        })?;
        let id = crate::audit::record(
            &tx,
            crate::audit::AuditKind::Reconcile,
            &actor,
            &format!("role:{}", role.name),
            crate::audit::AuditStatus::Ok,
            &format!("role_upserted by pack {pack_id}@{version}"),
        );
        if let Some(id) = id {
            audit_refs.push(id);
        }
    }

    // 3 · the procedures act — the procedure door's internals
    // (store_procedure writes its own Procedure audit row inside THIS tx).
    // The wizard-pack JSON rides the registered schema as CONTENT (READ
    // FIRST ②): procedure DATA, never a catalog entry, the drift gate
    // byte-green. The pack's content is screened exactly like /procedure.
    use crate::screen::ScreenResult;
    for (stem, value) in &pack.procedures {
        let title = procedure_title(stem);
        let content = serde_json::to_string_pretty(value).map_err(|e| ImportError {
            act: "procedures",
            message: e.to_string(),
        })?;
        let verdict = crate::screen::screen(&content, &title);
        if verdict == ScreenResult::Reject {
            return Err(ImportError {
                act: "procedures",
                message: format!("'{title}' was rejected by the input screen"),
            });
        }
        let quarantined = verdict == ScreenResult::Quarantine;
        let (root_id, _) = crate::service::procedure::store_procedure(
            &tx,
            &title,
            &content,
            pack.domain.as_ref().map(|d| d.name.as_str()),
            &[],
            quarantined,
            &[],
            &Some(actor.clone()),
            "operator",
        )
        .map_err(|e| ImportError {
            act: "procedures",
            message: e.to_string(),
        })?;
        let _ = root_id; // the act's evidence is the Procedure audit row
    }

    // 4 · the rules act — THE one rules door, second consumer (the door
    // round's promise): one Rules audit row per LANDED rule, the natural-key
    // revision law enforced by the door itself. A same-revision replay is
    // the C1 fixture's skip ("a replay inserts nothing") — a pack upgrade
    // carrying unchanged rules imports cleanly; a STALE rule still refuses
    // the whole import.
    for rule in &pack.rules {
        match crate::service::rules::insert_outcome(&tx, &actor, rule) {
            Ok(crate::service::rules::InsertOutcome::Landed(_)) => {}
            Ok(crate::service::rules::InsertOutcome::Current) => {}
            Err(e) => {
                return Err(ImportError {
                    act: "rules",
                    message: e,
                });
            }
        }
    }

    // 5 · the import act: the Plugin audit row, then the register row that
    // carries the plaintext id + version + hashes.
    let import_audit = crate::audit::record(
        &tx,
        crate::audit::AuditKind::Plugin,
        "operator",
        &format!("pack:{pack_id}"),
        crate::audit::AuditStatus::Ok,
        &format!(
            "import {pack_id}@{version} kinds={} archive_sha3:{} manifest_sha3:{} files:{}",
            pack.manifest
                .kinds
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(","),
            &archive_hash[..16.min(archive_hash.len())],
            &manifest_hash[..16.min(manifest_hash.len())],
            file_count
        ),
    );
    if let Some(id) = import_audit {
        audit_refs.push(id);
    }

    let kinds = pack
        .manifest
        .kinds
        .iter()
        .map(|k| k.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let rows_landed = plan(pack).len();
    match existing_register {
        Some(id) => {
            tx.execute(
                "UPDATE installed_packs SET version = ?2, kinds = ?3, manifest_hash = ?4,
                     archive_hash = ?5, file_count = ?6, import_audit_id = ?7,
                     imported_at = ?8
                 WHERE id = ?1",
                rusqlite::params![
                    id,
                    version,
                    kinds,
                    manifest_hash,
                    archive_hash,
                    file_count as i64,
                    import_audit,
                    now
                ],
            )
            .map_err(|e| ImportError {
                act: "register",
                message: e.to_string(),
            })?;
            let _ = id;
        }
        None => {
            tx.execute(
                "INSERT INTO installed_packs(pack_id, version, kinds, manifest_hash,
                     archive_hash, file_count, import_audit_id, imported_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    pack_id,
                    version,
                    kinds,
                    manifest_hash,
                    archive_hash,
                    file_count as i64,
                    import_audit,
                    now
                ],
            )
            .map_err(|e| ImportError {
                act: "register",
                message: e.to_string(),
            })?;
        }
    }

    tx.commit().map_err(|e| ImportError {
        act: "tx",
        message: e.to_string(),
    })?;

    Ok(ImportReceipt {
        register_id: existing_register.unwrap_or_else(|| conn.last_insert_rowid()),
        rows_landed,
        audit_refs,
    })
}

/// The installed register, oldest first: what `plugin list` reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPack {
    pub pack_id: String,
    pub version: String,
    pub kinds: Vec<String>,
    pub manifest_hash: String,
    pub archive_hash: String,
    pub file_count: i64,
    pub import_audit_id: Option<i64>,
    pub imported_at: i64,
}

pub fn list_installed(conn: &Connection) -> Result<Vec<InstalledPack>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT pack_id, version, kinds, manifest_hash, archive_hash, file_count,
                    import_audit_id, imported_at
             FROM installed_packs ORDER BY imported_at, pack_id",
        )
        .map_err(|e| format!("installed_packs read: {e} (has the migration run?)"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(InstalledPack {
                pack_id: r.get(0)?,
                version: r.get(1)?,
                kinds: r
                    .get::<_, String>(2)?
                    .split(',')
                    .map(str::to_string)
                    .collect(),
                manifest_hash: r.get(3)?,
                archive_hash: r.get(4)?,
                file_count: r.get(5)?,
                import_audit_id: r.get(6)?,
                imported_at: r.get(7)?,
            })
        })
        .map_err(|e| format!("installed_packs read: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("installed_packs read: {e}"))
}

/// blake3 hex of content — the register's content hashes (the provenance
/// layer's own hash law).
pub fn content_hash(bytes: &[u8]) -> String {
    hex::encode(blake3::hash(bytes).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::{PackKind, validate_pack};

    fn db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("db");
        crate::migration::run_migration(&mut conn, 512).expect("migrate fixture");
        conn
    }

    /// A minimal VALID pack's entries (the G-1 battery's clean pack, trimmed
    /// to domain+rules — the smallest mount the law admits).
    fn pack_entries() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            (
                "manifest.json",
                serde_json::json!({
                    "id": "support-pack", "version": "0.1.0",
                    "engine": ">=1.0.0", "kinds": ["domain", "rules"]
                })
                .to_string()
                .into_bytes(),
            ),
            (
                "domain.json",
                serde_json::json!({
                    "name": "support", "label": "Support desk", "retention_days": 90
                })
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
        ]
    }

    fn validated() -> ValidatedPack {
        let entries = pack_entries();
        let refs: Vec<(&str, Vec<u8>)> = entries.iter().map(|(k, v)| (*k, v.clone())).collect();
        validate_pack(&refs).expect("the fixture pack validates")
    }

    #[test]
    fn the_import_lands_every_declared_row_and_registers_the_pack() {
        let mut conn = db();
        let pack = validated();
        let receipt = apply(
            &mut conn,
            &pack,
            &content_hash(b"archive"),
            &content_hash(b"manifest"),
            3,
            1_000,
        )
        .expect("the import lands");

        assert_eq!(
            receipt.rows_landed, 2,
            "the plan's count: 1 domain + 1 rule"
        );
        // the register carries the plaintext id + version + hashes
        let installed = list_installed(&conn).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].pack_id, "support-pack");
        assert_eq!(installed[0].version, "0.1.0");
        assert_eq!(installed[0].kinds, vec!["domain", "rules"]);
        assert_eq!(installed[0].manifest_hash, content_hash(b"manifest"));
        // the domain act bound the profile-less domain's audit only; the
        // rule act is on the chain as kind rules; the import act as plugin
        let rules_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='rules'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rules_rows, 1);
        let plugin_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='plugin'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(plugin_rows, 1);
        let reconcile_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='reconcile'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reconcile_rows, 1, "the domain act is on the chain");
        // the rule row itself lives
        let stored = crate::service::rules::list(&conn).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].rule_key, "ticket_retention");
    }

    #[test]
    fn a_verbatim_reinstall_is_refused_and_a_higher_version_upgrades() {
        let mut conn = db();
        let pack = validated();
        apply(&mut conn, &pack, "a", "m", 3, 1_000).unwrap();

        // same id+version: REFUSED, nothing moves
        let err = apply(&mut conn, &pack, "a", "m", 3, 2_000).unwrap_err();
        assert_eq!(err.act, "register");
        assert!(err.message.contains("ALREADY INSTALLED"), "{err}");
        assert_eq!(list_installed(&conn).unwrap()[0].imported_at, 1_000);

        // a higher version: the register row updates in place
        let mut entries = pack_entries();
        entries[0] = (
            "manifest.json",
            serde_json::json!({
                "id": "support-pack", "version": "0.2.0",
                "engine": ">=1.0.0", "kinds": ["domain", "rules"]
            })
            .to_string()
            .into_bytes(),
        );
        let refs: Vec<(&str, Vec<u8>)> = entries.iter().map(|(k, v)| (*k, v.clone())).collect();
        let upgraded = validate_pack(&refs).unwrap();
        apply(&mut conn, &upgraded, "a2", "m2", 3, 3_000).unwrap();
        let installed = list_installed(&conn).unwrap();
        assert_eq!(installed.len(), 1, "one living row per pack");
        assert_eq!(installed[0].version, "0.2.0");
        assert_eq!(installed[0].imported_at, 3_000);
    }

    #[test]
    fn a_failed_act_rolls_the_whole_import_back() {
        let mut conn = db();
        // a pack whose rule collides with an ALREADY-STORED higher revision:
        // the rules door refuses, and NOTHING from this import survives
        crate::service::rules::insert(
            &conn,
            "operator",
            &crate::packs::RuleRow {
                jurisdiction: "ph".into(),
                subject: "support-tickets".into(),
                rule_key: "ticket_retention".into(),
                body: "pre-existing law at r5".into(),
                source_ref: "prior".into(),
                effective_at: 0,
                reviewed_at: None,
                expires_at: None,
                revision: 5,
            },
        )
        .unwrap();
        let pack = validated(); // offers ticket_retention at r1 — stale
        let err = apply(&mut conn, &pack, "a", "m", 3, 1_000).unwrap_err();
        assert_eq!(err.act, "rules");
        // the rollback: no register row, no domain audit row, no role rows
        assert!(list_installed(&conn).unwrap().is_empty());
        let reconcile: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='reconcile'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reconcile, 0, "the failed import's acts rolled back");
    }

    #[test]
    fn the_plan_and_the_lines_are_one_vocabulary() {
        let pack = validated();
        let rows = plan(&pack);
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0], PlanRow::Domain { .. }));
        assert_eq!(plan_line(&rows[0]), "domain support");
        assert_eq!(
            plan_line(&rows[1]),
            "rule ph:support-tickets:ticket_retention @r1"
        );
        let _ = PackKind::Domain; // the kind vocabulary rides the manifest
    }
}
