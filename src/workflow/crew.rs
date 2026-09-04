//! Crew — colleagues become visible.
//!
//! Presence WITHOUT a background worker: every mutating request upserts one
//! [`touch`] row per `(domain, principal)` inside the CALLER's existing
//! transaction (the WorkflowTx / `BEGIN IMMEDIATE` posture — a rolled-back
//! transition leaves no presence ghost). Reads compute TTL decay at read
//! time: active < 5 min, away < 30 min, offline beyond. The roster view
//! merges presence with the Watchbill shift ring (site) and the HITL-
//! maintained skills tags; it shows WHAT KIND of act, never case content —
//! only an opaque `current_case_ref` plus the activity kind ever leave the
//! server, and the DPO switch (`crew_config`) hides everything when off or
//! unreadable.

use rusqlite::{Connection, OptionalExtension, params};

/// Active window: a principal seen within this many seconds is `active`.
pub const ACTIVE_SECS: i64 = 5 * 60;
/// Away window: seen within this many seconds is `away`; older is `offline`.
pub const AWAY_SECS: i64 = 30 * 60;
/// Skills bounds — a tag is lowercase alnum + hyphen, 1..=32 chars, at most
/// 32 per principal (row-size + suggestion-routing sanity).
pub const MAX_SKILLS: usize = 32;
pub const MAX_SKILL_LEN: usize = 32;
/// Read cap for list surfaces (the Bound law).
pub const MAX_CREW_RETURNED: i64 = 500;
/// The closed activity vocabulary. Presence shows the KIND of act only.
/// The closed activity-kind vocabulary. Herald adds `channel`: presence fed
/// from mapped Slack/Teams operator activity — activity KINDS only, never
/// content, DPO-switch governed at both the write (touch skipped when off)
/// and the read (roster hidden when off).
pub const ACTIVITY_KINDS: [&str; 4] = ["cranking", "reviewing", "idle", "channel"];
/// The proposal kind that gates every skills change (HITL).
pub const KIND_SKILLS_UPDATE: &str = "crew_skills_update";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum PresenceState {
    Active,
    Away,
    Offline,
}

impl PresenceState {
    pub fn as_str(self) -> &'static str {
        match self {
            PresenceState::Active => "active",
            PresenceState::Away => "away",
            PresenceState::Offline => "offline",
        }
    }
}

/// TTL decay computed at READ time from the stored `ts` — no worker ages
/// rows out; a stale row simply decays through the bands.
pub fn decay(now: i64, ts: i64) -> PresenceState {
    let age = (now - ts).max(0);
    if age < ACTIVE_SECS {
        PresenceState::Active
    } else if age < AWAY_SECS {
        PresenceState::Away
    } else {
        PresenceState::Offline
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CrewMember {
    pub principal: String,
    pub state: PresenceState,
    /// What KIND of authenticated act was last observed (closed vocabulary).
    pub activity_kind: String,
    /// Opaque reference of the run/case being worked — never its content.
    pub current_case_ref: Option<String>,
    /// Site whose shift roster currently contains this principal (Watchbill).
    pub site: Option<String>,
    pub roles: Vec<String>,
    pub skills: Vec<String>,
}

/// A skills change candidate (the payload carried inside the proposal).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillsChange {
    /// The domain the tags live in — carried IN the proposal so the approval
    /// applies to exactly the domain that was proposed (never the approver's
    /// ambient default). Defaults to `global` for legacy payloads.
    #[serde(default = "default_change_domain")]
    pub domain: String,
    pub principal: String,
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

fn default_change_domain() -> String {
    "global".to_string()
}

#[derive(Debug)]
pub enum CrewError {
    InvalidActivity(String),
    InvalidPrincipal(String),
    InvalidSkills(String),
    TooManySkills,
    ProposalNotFound,
    ProposalNotPending,
    Database(String),
}

impl std::fmt::Display for CrewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CrewError::InvalidActivity(k) => {
                write!(f, "activity_kind must be one of {ACTIVITY_KINDS:?}: {k}")
            }
            CrewError::InvalidPrincipal(p) => {
                write!(f, "principal must be 1..=256 chars: {p}")
            }
            CrewError::InvalidSkills(e) => write!(f, "invalid skill tag: {e}"),
            CrewError::TooManySkills => write!(f, "at most {MAX_SKILLS} skills per principal"),
            CrewError::ProposalNotFound => write!(f, "proposal not found"),
            CrewError::ProposalNotPending => write!(f, "proposal is not pending"),
            CrewError::Database(e) => write!(f, "database error: {e}"),
        }
    }
}

fn db_err(e: rusqlite::Error) -> CrewError {
    CrewError::Database(e.to_string())
}

/// Role-badge snapshot bound: at most 16 roles of 64 visible chars each —
/// the JWT claim is server-signed, but the stored row stays size-bounded
/// regardless of what a future issuer puts in the claim.
fn sanitize_roles(roles: &[String]) -> Vec<String> {
    roles
        .iter()
        .take(16)
        .map(|r| crate::strip_invisible::strip_invisible(&r.chars().take(64).collect::<String>()))
        .filter(|r| !r.is_empty())
        .collect()
}

pub fn is_valid_skill(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_SKILL_LEN
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn validate_change(change: &SkillsChange) -> Result<(), CrewError> {
    let d = &change.domain;
    if d.is_empty()
        || d.len() > 63
        || !d
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(CrewError::InvalidSkills(format!(
            "domain label invalid: {d}"
        )));
    }
    if change.principal.is_empty() || change.principal.len() > 256 {
        return Err(CrewError::InvalidPrincipal(change.principal.clone()));
    }
    for s in change.add.iter().chain(change.remove.iter()) {
        if !is_valid_skill(s) {
            return Err(CrewError::InvalidSkills(s.clone()));
        }
    }
    Ok(())
}

/// Pre-flight validation for the proposal endpoint — the same rule set runs
/// again inside the applying transaction (approval-time is authoritative).
pub fn apply_skills_change_probe(change: &SkillsChange) -> Result<(), CrewError> {
    validate_change(change)
}

/// The DPO switch. Absent row = enabled (presence is the default posture);
/// ANY read failure = disabled — presence fails open to HIDDEN, never to
/// more visibility than the operator configured.
pub fn presence_enabled(conn: &Connection, domain: &str) -> bool {
    match conn.query_row(
        "SELECT presence_enabled FROM crew_config WHERE domain = ?1",
        params![domain],
        |r| r.get::<_, i64>(0),
    ) {
        Ok(v) => v != 0,
        Err(rusqlite::Error::QueryReturnedNoRows) => true,
        // ANY other read failure hides the crew — visibility must never
        // default ON when its governing config cannot be trusted.
        Err(_) => false,
    }
}

/// Set the DPO switch (caller provides the tx; audited by the caller).
pub fn set_presence_enabled(
    conn: &Connection,
    domain: &str,
    enabled: bool,
    now: i64,
) -> Result<(), CrewError> {
    conn.execute(
        "INSERT INTO crew_config(domain, presence_enabled) VALUES (?1, ?2)
         ON CONFLICT(domain) DO UPDATE SET presence_enabled = ?2",
        params![domain, i64::from(enabled)],
    )
    .map_err(db_err)?;
    // A config row written before the table existed is impossible; keep `now`
    // in the signature so callers pass their tx timestamp explicitly (the
    // audit row and the config flip commit together).
    let _ = now;
    Ok(())
}

/// Upsert presence INSIDE the caller's transaction. No worker, no heartbeat:
/// a request that never happens leaves no row, and a transaction that rolls
/// back takes its presence bump with it. Roles ride the upsert (the JWT
/// claim snapshot — role badges are what the token asserted last).
pub fn touch(
    conn: &Connection,
    domain: &str,
    principal: &str,
    activity_kind: &str,
    current_case_ref: Option<&str>,
    roles: &[String],
    now: i64,
) -> Result<(), CrewError> {
    if !ACTIVITY_KINDS.contains(&activity_kind) {
        return Err(CrewError::InvalidActivity(activity_kind.to_string()));
    }
    if principal.is_empty() || principal.len() > 256 {
        return Err(CrewError::InvalidPrincipal(principal.to_string()));
    }
    let roles_json = serde_json::to_string(&sanitize_roles(roles))
        .map_err(|e| CrewError::Database(e.to_string()))?;
    let case_ref = current_case_ref.map(|r| r.chars().take(128).collect::<String>());
    conn.execute(
        "INSERT INTO presence(domain, principal, ts, activity_kind, current_case_ref, roles_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(domain, principal) DO UPDATE SET
            ts = ?3, activity_kind = ?4, current_case_ref = ?5, roles_json = ?6",
        params![domain, principal, now, activity_kind, case_ref, roles_json],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Presence rides the caller's transaction — a mutating request is its own
/// beacon, and a rolled-back tx leaves no ghost. Best-effort (presence
/// never gates the work): a failed touch is a loud warn, never an error.
pub(crate) fn touch_cranking(conn: &Connection, domain: &str, actor: &str, case_ref: Option<&str>) {
    if let Err(e) = touch(
        conn,
        domain,
        actor,
        "cranking",
        case_ref,
        &[],
        chrono::Utc::now().timestamp(),
    ) {
        tracing::warn!("presence touch failed: {e}");
    }
}

fn roles_from(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(json).unwrap_or_default()
}

/// The roster view: presence × shift-ring site × skills, TTL-decayed at
/// read. When presence is disabled (or the config cannot be trusted) the
/// roster is EMPTY — hidden, not an error. Members carry only kinds and
/// opaque refs, never case content.
pub fn roster(conn: &Connection, domain: &str, now: i64) -> Result<Vec<CrewMember>, CrewError> {
    if !presence_enabled(conn, domain) {
        return Ok(Vec::new());
    }
    struct Row {
        principal: String,
        ts: i64,
        activity_kind: String,
        current_case_ref: Option<String>,
        roles_json: String,
    }
    let mut stmt = conn
        .prepare(
            "SELECT principal, ts, activity_kind, current_case_ref, roles_json
             FROM presence WHERE domain = ?1
             ORDER BY ts DESC LIMIT ?2",
        )
        .map_err(db_err)?;
    let rows: Vec<Row> = stmt
        .query_map(params![domain, MAX_CREW_RETURNED], |r| {
            Ok(Row {
                principal: r.get(0)?,
                ts: r.get(1)?,
                activity_kind: r.get(2)?,
                current_case_ref: r.get(3)?,
                roles_json: r.get(4)?,
            })
        })
        .map_err(db_err)?
        .flatten()
        .collect();
    drop(stmt);
    // Watchbill join: the site whose CURRENT shift roster names the member.
    let shifts = crate::workflow::shifts::list_shifts(conn, domain)
        .map_err(|e| CrewError::Database(format!("shift ring unreadable: {e}")))?;
    let site_of = |principal: &str| -> Option<String> {
        shifts
            .iter()
            .filter(|s| {
                s.start_epoch <= now && now < s.end_epoch && s.roster.iter().any(|p| p == principal)
            })
            .max_by_key(|s| s.start_epoch)
            .map(|s| s.site.clone())
    };
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let mut stmt = conn
            .prepare(
                "SELECT skill FROM principal_skills WHERE domain = ?1 AND principal = ?2
                 ORDER BY skill",
            )
            .map_err(db_err)?;
        let skills: Vec<String> = stmt
            .query_map(params![domain, r.principal], |row| row.get(0))
            .map_err(db_err)?
            .flatten()
            .collect();
        drop(stmt);
        // Every emitted string passes the invisible-strip seam (the fence
        // holds at the VIEW, not call-site discipline): a stored principal id
        // or case ref carrying zero-width/bidi characters cannot smuggle a
        // fence marker or instruction through the roster into any LLM-facing
        // consumer. Activity kind needs no strip (closed vocabulary, checked
        // again below); site comes from operator-declared shifts.
        let principal = crate::strip_invisible::strip_invisible(&r.principal);
        if principal.is_empty() {
            continue;
        }
        let current_case_ref = r
            .current_case_ref
            .as_deref()
            .map(crate::strip_invisible::strip_invisible)
            .filter(|s| !s.is_empty());
        let activity_kind = if ACTIVITY_KINDS.contains(&r.activity_kind.as_str()) {
            r.activity_kind.clone()
        } else {
            "idle".to_string()
        };
        out.push(CrewMember {
            state: decay(now, r.ts),
            activity_kind,
            current_case_ref,
            site: site_of(&principal),
            roles: roles_from(&r.roles_json),
            skills,
            principal,
        });
    }
    Ok(out)
}

/// Apply an approved skills change IN the caller's transaction: adds then
/// removes, idempotent via the composite primary key. The caller CASes the
/// proposal to `approved` FIRST (see [`apply_proposal`] for the combined
/// form used by both the approval gate and tests).
pub fn apply_skills_change(
    conn: &Connection,
    domain: &str,
    change: &SkillsChange,
    now: i64,
) -> Result<usize, CrewError> {
    validate_change(change)?;
    let existing: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM principal_skills WHERE domain = ?1 AND principal = ?2",
            params![domain, change.principal],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    let would_add = change
        .add
        .iter()
        .filter(|s| !change.remove.contains(s))
        .count() as i64;
    if existing + would_add > MAX_SKILLS as i64 {
        return Err(CrewError::TooManySkills);
    }
    let mut n = 0;
    for s in &change.add {
        n += conn
            .execute(
                "INSERT OR IGNORE INTO principal_skills(domain, principal, skill, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![domain, change.principal, s, now],
            )
            .map_err(db_err)?;
    }
    for s in &change.remove {
        n += conn
            .execute(
                "DELETE FROM principal_skills WHERE domain = ?1 AND principal = ?2 AND skill = ?3",
                params![domain, change.principal, s],
            )
            .map_err(db_err)?;
    }
    Ok(n)
}

fn parse_change(content: &str) -> Result<SkillsChange, CrewError> {
    serde_json::from_str::<SkillsChange>(content)
        .map_err(|e| CrewError::InvalidSkills(e.to_string()))
}

/// The WFM skills feed (interop boundary — COPC alignment, not
/// reimplementation): the domain's skill registry as a stable, bounded,
/// ordered read. Skills stay HITL-maintained; this only exposes them.
pub const MAX_SKILLS_FEED_ROWS: i64 = 1000;

pub fn list_skills(conn: &Connection, domain: &str) -> Result<Vec<(String, String)>, CrewError> {
    let mut stmt = conn
        .prepare(
            "SELECT principal, skill FROM principal_skills WHERE domain = ?1
             ORDER BY principal, skill LIMIT ?2",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![domain, MAX_SKILLS_FEED_ROWS], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Crew routing: worktype × skills tags → the colleague board. A principal
/// sits on the board for a worktype when their HITL-maintained tags cover
/// EVERY required tag; a worktype with no required tags routes to everyone.
/// Deterministic, order-preserving over the (already ordered) skills rows,
/// deduped — pure data shaping over the WFM feed, no presence read.
pub fn board_for_worktype(skills: &[(String, String)], required: &[&str]) -> Vec<String> {
    let mut board: Vec<String> = Vec::new();
    for principal in skills.iter().map(|(p, _)| p) {
        if board.iter().any(|b| b == principal) {
            continue;
        }
        let owned: Vec<&str> = skills
            .iter()
            .filter(|(p, _)| p == principal)
            .map(|(_, s)| s.as_str())
            .collect();
        let covers = required.iter().all(|r| owned.contains(r));
        if covers {
            board.push(principal.clone());
        }
    }
    board
}

/// The approval-side primitive: CAS a PENDING `crew_skills_update` proposal
/// to `approved` and apply its change in the SAME transaction. Returns the
/// applied row count. A proposal that lost a race (no longer pending) is a
/// conflict, never a double-apply.
pub fn apply_proposal(
    conn: &Connection,
    proposal_id: i64,
    _fallback_domain: &str,
    now: i64,
) -> Result<usize, CrewError> {
    let (status, content): (String, String) = conn
        .query_row(
            "SELECT status, content FROM proposals WHERE id = ?1 AND kind = ?2",
            params![proposal_id, KIND_SKILLS_UPDATE],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db_err)?
        .ok_or(CrewError::ProposalNotFound)?;
    if status != "pending" {
        return Err(CrewError::ProposalNotPending);
    }
    let change = parse_change(&content)?;
    let n = apply_skills_change(conn, &change.domain, &change, now)?;
    let updated = conn
        .execute(
            "UPDATE proposals SET status = 'approved', decided_at = ?1
             WHERE id = ?2 AND status = 'pending'",
            params![now, proposal_id],
        )
        .map_err(db_err)?;
    if updated == 0 {
        return Err(CrewError::ProposalNotPending);
    }
    Ok(n)
}

/// File ONE `crew_skills_update` proposal: the raw INSERT + id resolution
/// inside the CALLER'S tx. Probe validation stays at the caller; the audit
/// row (same tx) stays at the call site, adjacent to the write it
/// evidences. The change's target domain is stamped on the row (Triage) so
/// the review queue scopes the proposal to the domain whose roster it
/// edits — the same domain the filing caller was authorized against.
pub(crate) fn file_skills_proposal(
    tx: &Connection,
    content: &str,
    owner: &str,
    domain: &str,
    now: i64,
) -> rusqlite::Result<i64> {
    tx.query_row(
        "INSERT INTO proposals(kind, content, novelty, salience, created_at, owner, domain)
         VALUES (?1, ?2, 0.5, 0.5, ?3, ?4, ?5) RETURNING id",
        params![KIND_SKILLS_UPDATE, content, now, owner, domain],
        |r| r.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::frontdoor::worktype_skills;

    #[test]
    fn crew_board_routes_by_worktype_tags() {
        let skills = vec![
            ("ana".to_string(), "returns".to_string()),
            ("ana".to_string(), "warranty".to_string()),
            ("bob".to_string(), "returns".to_string()),
            ("cyd".to_string(), "safety".to_string()),
            ("cyd".to_string(), "compliance".to_string()),
        ];
        // warranty_claim requires BOTH tags: only ana qualifies.
        let req = worktype_skills("warranty_claim");
        assert_eq!(board_for_worktype(&skills, req), vec!["ana".to_string()]);
        // safety_recall requires safety + compliance: cyd.
        let req = worktype_skills("safety_recall");
        assert_eq!(board_for_worktype(&skills, req), vec!["cyd".to_string()]);
        // No required tags (unknown worktype) → everyone, deterministic order.
        assert_eq!(
            board_for_worktype(&skills, worktype_skills("mystery")),
            vec!["ana".to_string(), "bob".to_string(), "cyd".to_string()]
        );
        // Nobody holds the tags → an empty board, never a guess.
        assert!(board_for_worktype(&skills, &["quantum"]).is_empty());
    }

    fn seed() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("open");
        run_migration(&mut conn, 1).expect("migration");
        conn
    }

    fn insert_proposal(conn: &Connection, content: &str) -> i64 {
        conn.query_row(
            "INSERT INTO proposals(kind, content, novelty, salience, created_at)
             VALUES (?1, ?2, 0.9, 0.5, 100) RETURNING id",
            params![KIND_SKILLS_UPDATE, content],
            |r| r.get(0),
        )
        .expect("insert proposal")
    }

    #[test]
    fn presence_upserts_ride_existing_transactions_no_worker() {
        let mut conn = seed();
        // The upsert runs on the CALLER's transaction: rolling the caller's
        // tx back takes the presence bump with it — there is no separate
        // writer that could have committed it.
        {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .expect("tx");
            touch(
                &tx,
                "global",
                "alice",
                "cranking",
                Some("run:7"),
                &["ops".into()],
                1000,
            )
            .expect("touch");
            touch(&tx, "global", "bob", "reviewing", None, &[], 1001).expect("touch");
            tx.rollback().expect("rollback");
        }
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM presence", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 0, "rolled-back tx leaves no presence ghost");
        {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .expect("tx");
            touch(
                &tx,
                "global",
                "alice",
                "cranking",
                Some("run:7"),
                &["ops".into()],
                1000,
            )
            .expect("touch");
            tx.commit().expect("commit");
        }
        // Re-touch UPSERTS (one row per principal), refreshing kind + ref.
        touch(
            &conn,
            "global",
            "alice",
            "reviewing",
            None,
            &["ops".into()],
            1100,
        )
        .expect("re-touch");
        let (kind, ts): (String, i64) = conn
            .query_row(
                "SELECT activity_kind, ts FROM presence WHERE principal = 'alice'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("read");
        assert_eq!((kind.as_str(), ts), ("reviewing", 1100));
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM presence", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 1);
        // Unknown activity kinds refuse before any write.
        assert!(matches!(
            touch(&conn, "global", "alice", "keystrokes", None, &[], 1200),
            Err(CrewError::InvalidActivity(_))
        ));
    }

    #[test]
    fn presence_decays_by_ttl_at_read() {
        let conn = seed();
        let now = 10_000;
        for (principal, ts) in [
            ("a", 10_000),
            ("b", now - ACTIVE_SECS),
            ("c", now - AWAY_SECS),
        ] {
            touch(&conn, "global", principal, "idle", None, &[], ts).expect("touch");
        }
        let crew = roster(&conn, "global", now).expect("roster");
        let state_of = |p: &str| {
            crew.iter()
                .find(|m| m.principal == p)
                .map(|m| m.state)
                .expect("member")
        };
        assert_eq!(state_of("a"), PresenceState::Active);
        assert_eq!(state_of("b"), PresenceState::Away);
        assert_eq!(state_of("c"), PresenceState::Offline);
        // Decay boundaries themselves.
        assert_eq!(decay(now, now - ACTIVE_SECS + 1), PresenceState::Active);
        assert_eq!(decay(now, now - AWAY_SECS + 1), PresenceState::Away);
        assert_eq!(decay(now, now), PresenceState::Active);
    }

    #[test]
    fn roster_never_exposes_case_content() {
        let conn = seed();
        // The DPO switch OFF → hidden even with live presence rows.
        set_presence_enabled(&conn, "global", false, 1).expect("config");
        touch(
            &conn,
            "global",
            "alice",
            "cranking",
            Some("run:42"),
            &[],
            900,
        )
        .expect("touch");
        let crew = roster(&conn, "global", 1000).expect("roster");
        assert!(crew.is_empty(), "disabled presence hides everyone");
        // Config unreadable → also hidden (fail-open to hidden). Simulate by
        // dropping the table out from under the reader on a scratch conn.
        let broken = seed();
        broken
            .execute_batch("DROP TABLE crew_config;")
            .expect("drop config");
        assert!(
            !presence_enabled(&broken, "global"),
            "unreadable config = hidden"
        );

        set_presence_enabled(&conn, "global", true, 2).expect("config");
        let crew = roster(&conn, "global", 1000).expect("roster");
        assert_eq!(crew.len(), 1);
        let m = &crew[0];
        assert_eq!(m.current_case_ref.as_deref(), Some("run:42"));
        assert_eq!(m.activity_kind, "cranking");
        // The member projection carries ONLY the closed vocabulary + opaque
        // ref — no free-text field exists that could smuggle case content.
        let json = serde_json::to_value(m).expect("serialize");
        let obj = json.as_object().expect("object");
        for key in obj.keys() {
            assert!(
                [
                    "principal",
                    "state",
                    "activity_kind",
                    "current_case_ref",
                    "site",
                    "roles",
                    "skills"
                ]
                .contains(&key.as_str()),
                "unexpected roster field: {key}"
            );
        }
        // A principal id planted with invisible characters cannot smuggle a
        // fence marker or instruction through the roster view.
        touch(
            &conn,
            "global",
            "ev\u{200B}il\u{FEFF}",
            "idle",
            None,
            &[],
            950,
        )
        .expect("touch");
        let crew = roster(&conn, "global", 1000).expect("roster");
        let principals: Vec<&str> = crew.iter().map(|m| m.principal.as_str()).collect();
        assert!(
            principals.iter().all(|p| p
                .chars()
                .all(|c| !c.is_control() && c != '\u{200B}' && c != '\u{FEFF}')),
            "invisible characters survived the read seam: {principals:?}"
        );
    }

    #[test]
    fn skills_proposal_applies_to_its_own_domain_not_the_approvers() {
        let conn = seed();
        let p = insert_proposal(
            &conn,
            &serde_json::json!({
                "domain": "acme",
                "principal": "bob",
                "add": ["networking"],
                "remove": []
            })
            .to_string(),
        );
        // Approval runs on the global pool; the tags MUST land under the
        // domain carried inside the proposal, never the caller's ambient one.
        apply_proposal(&conn, p, "global", 100).expect("apply");
        assert_eq!(
            count_where(&conn, "acme"),
            1,
            "tags land under the proposed domain"
        );
        assert_eq!(count_where(&conn, "global"), 0);
    }

    fn count_where(conn: &Connection, domain: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM principal_skills WHERE domain = ?1",
            params![domain],
            |r| r.get(0),
        )
        .expect("count")
    }

    #[test]
    fn skills_changes_are_proposal_gated() {
        let conn = seed();
        let content = serde_json::json!({
            "principal": "bob",
            "add": ["networking", "voip"],
            "remove": []
        })
        .to_string();
        let id = insert_proposal(&conn, &content);
        // Before approval: no direct write path has run — no skills exist.
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM principal_skills", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 0, "proposal creation alone writes no skills");
        // Approval applies the change and CASes the proposal in one tx.
        let applied = apply_proposal(&conn, id, "global", 200).expect("apply");
        assert_eq!(applied, 2);
        let skills: Vec<String> = conn
            .prepare("SELECT skill FROM principal_skills WHERE principal='bob' ORDER BY skill")
            .expect("stmt")
            .query_map([], |r| r.get(0))
            .expect("rows")
            .flatten()
            .collect();
        assert_eq!(skills, vec!["networking".to_string(), "voip".to_string()]);
        let status: String = conn
            .query_row("SELECT status FROM proposals WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .expect("status");
        assert_eq!(status, "approved");
        // Replay is refused (already decided) — never a double-apply.
        assert!(matches!(
            apply_proposal(&conn, id, "global", 300),
            Err(CrewError::ProposalNotPending)
        ));
        // Invalid tags refuse BEFORE any row lands.
        let bad = insert_proposal(
            &conn,
            &serde_json::json!({"principal":"eve","add":["Bad Tag!"],"remove":[]}).to_string(),
        );
        assert!(matches!(
            apply_proposal(&conn, bad, "global", 400),
            Err(CrewError::InvalidSkills(_))
        ));
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM principal_skills WHERE principal='eve'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(n, 0);
        // The cap holds across sequential approvals.
        for i in 0..MAX_SKILLS + 5 {
            let p = insert_proposal(
                &conn,
                &serde_json::json!({"principal":"carol","add":[format!("s{i}")],"remove":[]})
                    .to_string(),
            );
            match apply_proposal(&conn, p, "global", 500) {
                Ok(_) => {}
                Err(CrewError::TooManySkills) => {
                    assert!(i >= MAX_SKILLS, "cap fired early at {i}");
                    break;
                }
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
    }

    #[test]
    fn roster_joins_watchbill_site_and_skills() {
        use crate::workflow::shifts::{ShiftDraft, insert_shift};
        let conn = seed();
        let roster_ids = ["alice".to_string()];
        insert_shift(
            &conn,
            &ShiftDraft {
                domain: "global",
                site: "manila",
                tz: "UTC",
                start_epoch: 0,
                end_epoch: 10_000,
                overlap_minutes: 0,
                roster: &roster_ids,
            },
        )
        .expect("shift");
        touch(
            &conn,
            "global",
            "alice",
            "cranking",
            None,
            &["bpo-ops".into()],
            500,
        )
        .expect("t");
        apply_proposal(
            &conn,
            insert_proposal(
                &conn,
                &serde_json::json!({"principal":"alice","add":["networking"],"remove":[]})
                    .to_string(),
            ),
            "global",
            600,
        )
        .expect("skills");
        let crew = roster(&conn, "global", 1000).expect("roster");
        assert_eq!(crew.len(), 1);
        assert_eq!(crew[0].site.as_deref(), Some("manila"));
        assert_eq!(crew[0].skills, vec!["networking".to_string()]);
        assert_eq!(crew[0].roles, vec!["bpo-ops".to_string()]);
        // Outside her shift: no site badge.
        let crew = roster(&conn, "global", 20_000).expect("roster");
        assert_eq!(crew[0].site, None);
    }
}
