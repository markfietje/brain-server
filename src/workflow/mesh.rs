//! The Mesh core: agents as named colleagues inside one deployment.
//!
//! Three governed primitives, all deterministic and HITL-shaped:
//!
//! - **Agent Cards** — the A2A-standard JSON shape (name, description,
//!   capabilities) as an Ed25519-signed manifest per agent principal,
//!   signed with the UMP operator key at provisioning and RE-VERIFIED at
//!   every use point (read + delegation). A card whose signature no longer
//!   matches the operator key refuses loudly — fail closed.
//! - **Delegation** — agent→agent task events on a run's lineage: the
//!   request names the target's VERIFIED card; results return through the
//!   same lineage. Task/result CONTENT lives in the `delegations` table;
//!   lineage payloads carry ids + actors only (the Channel law).
//! - **Working sets** — a pure arbiter mapping (base domain, agent) to the
//!   agent's scratch domain name. Agent writes land there; promotion into
//!   the shared domain stays behind the existing proposal gate.

use crate::audit::AuditStatus;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

pub const TOPIC_REQUEST: &str = "delegation/request";
pub const TOPIC_RESULT: &str = "delegation/result";
/// The revocation-drain lineage marker — evidence the in-flight
/// run was cancelled BECAUSE its owning principal was revoked (the drill's
/// "drain observed in events" reading rides this topic).
pub const TOPIC_REVOKED: &str = "delegation/revoked";

/// The operator-key generation counter (schema_meta; 0 = first generation).
/// Bumped by `brain key rotate`; sign_card stamps it onto new cards;
/// verify_card uses it to pick the key deterministically.
pub(crate) fn operator_key_generation(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COALESCE((SELECT value FROM schema_meta WHERE key = 'operator_key_generation'), '0')",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse::<i64>().ok())
    .unwrap_or(0)
}

pub const STATE_REQUESTED: &str = "requested";
pub const STATE_COMPLETED: &str = "completed";

/// The terminal status the EXISTING cancel path writes
/// (`workflow::state::cas_update` with this status — the same path
/// `PUT /workflow/runs/{id}/state` serves; relay handover already refuses
/// any non-`active` run, so a cancelled run is drained for real).
pub const STATE_CANCELLED: &str = "cancelled";

/// Bounds for the revocation reason (the bounds law). Screened at the
/// handler seam like every operator free-text field.
pub const MAX_REVOKE_REASON_LEN: usize = 500;

pub const MAX_NAME_LEN: usize = 200;
pub const MAX_DESCRIPTION_LEN: usize = 1000;
pub const MAX_CAPABILITIES_LEN: usize = 4000;
pub const MAX_TASK_LEN: usize = 4000;

/// Per-run delegation ceiling — a run cannot be drowned in agent work
/// orders; evidence is refused, never drop-oldest-deleted.
pub const MAX_DELEGATIONS_PER_RUN: i64 = 64;

/// Principal-id bound shared with the Channel (same identity vocabulary).
pub const MAX_PRINCIPAL_LEN: usize = super::channel::MAX_PRINCIPAL_LEN;

#[derive(Debug)]
pub enum MeshError {
    /// Provisioning requires the operator key; absent key refuses loudly.
    NoOperatorKey,
    /// The stored card no longer verifies against the operator key.
    CardTampered(String),
    /// No card provisioned for this principal in this domain.
    CardUnknown(String),
    /// Input failed its bounds/shape check (`what`, `why`).
    InvalidInput(&'static str, &'static str),
    /// The run's delegation ceiling is reached.
    DelegationsFull,
    /// The delegation row does not exist on this run.
    NotFound(&'static str),
    /// Only the delegated agent may submit a result.
    NotDelegatee(String),
    /// The result was already submitted (CAS replay).
    AlreadyCompleted,
    /// The principal is revoked (ASI03/07 kill-switch): the card, the
    /// delegation, or the result is refused BEFORE anything else runs.
    PrincipalRevoked(String),
    Database(String),
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeshError::NoOperatorKey => {
                write!(f, "no operator signing key — cards refuse to provision")
            }
            MeshError::CardTampered(p) => {
                write!(f, "agent card for {p} fails signature verification")
            }
            MeshError::CardUnknown(p) => write!(f, "no agent card for {p}"),
            MeshError::InvalidInput(w, why) => write!(f, "invalid {w}: {why}"),
            MeshError::DelegationsFull => write!(f, "run reached its delegation ceiling"),
            MeshError::NotFound(w) => write!(f, "{w} not found"),
            MeshError::NotDelegatee(p) => write!(f, "only the delegated agent may submit: {p}"),
            MeshError::AlreadyCompleted => write!(f, "result already submitted"),
            MeshError::PrincipalRevoked(p) => {
                write!(
                    f,
                    "principal {p} is revoked — card and delegation refuse closed"
                )
            }
            MeshError::Database(m) => write!(f, "{m}"),
        }
    }
}

impl From<rusqlite::Error> for MeshError {
    fn from(e: rusqlite::Error) -> Self {
        MeshError::Database(e.to_string())
    }
}

/// True when `principal` sits in `revoked_principals` — the kill-switch
/// read every identity decision consults. Pure table read; errors read as
/// NOT revoked only for a genuinely missing table (fresh DB), never for a
/// query failure (those propagate — silence is never certified).
pub(crate) fn is_revoked(conn: &Connection, principal: &str) -> Result<bool, MeshError> {
    let hit: Option<String> = conn
        .query_row(
            "SELECT principal FROM revoked_principals WHERE principal = ?1",
            params![principal],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| MeshError::Database(e.to_string()))?;
    Ok(hit.is_some())
}

/// True when `principal` is a NAME this deployment has actually seen — an
/// agent card, the opaque loopback agent, crew presence/skills, either side
/// of a delegation, or a prior revocation. The revoke seam reports this as
/// ADVISORY (`known:false` + `warning`) — it never refuses on it: the
/// kill-switch writes unconditionally (a JWT `sub` with no row
/// anywhere is still live, and refusing it broke the kill-switch for exactly
/// those identities). A typo'd revoke (`"agent"` for `"agent@loopback"`)
/// still warns loudly naming the loopback agent — the fourth-pass confusion
/// is caught, never certified, and never a refusal.
pub(crate) fn principal_known(conn: &Connection, principal: &str) -> Result<bool, MeshError> {
    // The opaque Twokeys agent principal has no card and may have no runs —
    // it is known by construction (config.rs line-2 authentication).
    if principal == crate::auth::AGENT_LOOPBACK_SUB {
        return Ok(true);
    }
    let known: bool = conn
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM agent_cards WHERE principal = ?1
               UNION ALL
               SELECT 1 FROM presence WHERE principal = ?1
               UNION ALL
               SELECT 1 FROM principal_skills WHERE principal = ?1
               UNION ALL
               SELECT 1 FROM delegations WHERE from_principal = ?1 OR to_principal = ?1
               UNION ALL
               SELECT 1 FROM revoked_principals WHERE principal = ?1)",
            params![principal],
            |r| r.get(0),
        )
        .map_err(|e| MeshError::Database(e.to_string()))?;
    Ok(known)
}

/// Revoke a principal: the ASI03/07 kill-switch. One upsert (latest
/// revocation wins), one hash-chained global audit row, and the drain —
/// every ACTIVE run where the principal owns in-flight (`requested`)
/// delegation work is cancelled through the EXISTING cancel path
/// ([`crate::workflow::state::cas_update`] → status `cancelled`), each with
/// its own run-scoped audit row + `delegation/revoked` lineage event. All of
/// it inside the CALLER's transaction: a revocation and its evidence commit
/// or roll back together. Returns the number of runs drained.
///
/// A drain whose CAS races a concurrent state advance skips that run (the
/// decision-time revocation re-checks below still refuse every later step) —
/// the revocation itself is the primary act and never rolls back for it.
pub(crate) fn revoke_principal(
    conn: &Connection,
    principal: &str,
    reason: &str,
    revoked_by: &str,
    now: i64,
) -> Result<usize, MeshError> {
    if principal.is_empty() || principal.len() > MAX_PRINCIPAL_LEN {
        return Err(MeshError::InvalidInput("principal", "1..=256 chars"));
    }
    if reason.len() > MAX_REVOKE_REASON_LEN {
        return Err(MeshError::InvalidInput("reason", "≤500 chars"));
    }
    conn.execute(
        "INSERT INTO revoked_principals(principal, revoked_at, reason, revoked_by)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(principal) DO UPDATE SET
             revoked_at = excluded.revoked_at,
             reason = excluded.reason,
             revoked_by = excluded.revoked_by",
        params![principal, now, reason, revoked_by],
    )
    .map_err(|e| MeshError::Database(e.to_string()))?;
    crate::audit::record(
        conn,
        crate::audit::AuditKind::Auth,
        revoked_by,
        &format!("principal:{principal}"),
        crate::audit::AuditStatus::Ok,
        &format!("revoke:{reason}"),
    );
    // The drain: active runs whose in-flight delegations this principal owns.
    // PAGED (the old single LIMIT 200 silently abandoned run 201+): up to
    // 10 pages of 200, then a loud `drain_incomplete` row naming the
    // remainder. Pages advance because the cancels run INSIDE the loop —
    // each CAS-cancel moves its run out of the active set, so the next
    // page's predicate naturally walks forward (the 2026-09-11 fix: the
    // old shape collected pages WITHOUT cancelling in between, so pages
    // 2..N re-read the identical first 200 rows and the drain capped at
    // 200 DISTINCT victims no matter the budget).
    const DRAIN_PAGE: i64 = 200;
    const DRAIN_MAX_PAGES: usize = 10;
    let mut victims: Vec<(i64, String, i64)> = Vec::new();
    let mut pages = 0usize;
    loop {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT d.run_id, r.state_json, r.state_revision
                   FROM delegations d JOIN workflow_runs r ON r.id = d.run_id
                  WHERE d.from_principal = ?1 AND d.state = ?2 AND r.status = 'active'
                  ORDER BY d.run_id LIMIT ?3",
            )
            .map_err(|e| MeshError::Database(e.to_string()))?;
        let page: Vec<(i64, String, i64)> = stmt
            .query_map(params![principal, STATE_REQUESTED, DRAIN_PAGE], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .map_err(|e| MeshError::Database(e.to_string()))?
            .collect::<Result<_, _>>()
            .map_err(|e| MeshError::Database(e.to_string()))?;
        drop(stmt);
        let full = page.len() == DRAIN_PAGE as usize;
        // Cancel THIS page before querying the next — the cancels are what
        // advance the predicate (CAS-stale rows stay for the recount).
        for (run_id, state_json, revision) in page {
            let cancelled = crate::workflow::state::cas_update(
                conn,
                run_id,
                revision,
                &state_json,
                STATE_CANCELLED,
                now,
            );
            if cancelled.is_err() {
                // CAS-stale: the run advanced concurrently. Leave it — every
                // later delegation/result decision still re-checks revocation.
                continue;
            }
            let _ = super::outbox::append_lineage(
                conn,
                run_id,
                TOPIC_REVOKED,
                &serde_json::json!({
                    "action": "revocation_drain",
                    "principal": principal,
                })
                .to_string(),
                &format!("revoked:{principal}:{run_id}"),
                now,
            );
            super::audit_write(
                conn,
                run_id,
                &format!("run:{run_id}"),
                crate::audit::AuditStatus::Ok,
                &format!("revocation drain (owner {principal})"),
            );
            victims.push((run_id, state_json, revision));
        }
        pages += 1;
        if !full || pages >= DRAIN_MAX_PAGES {
            break;
        }
    }
    // The loud remainder: a drain capped by the page budget must NAME what
    // it could not finish (the old single-page drain was silent about runs
    // past 200). The row lands on the hash-chained audit trail (kind Auth,
    // the revocation register's evidence) + the error log stream. The
    // recount re-runs the SAME predicate post-cancel, so CAS-stale rows
    // that stayed active are counted honestly.
    let drained = victims.len();
    if pages >= DRAIN_MAX_PAGES {
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT d.run_id) FROM delegations d
                   JOIN workflow_runs r ON r.id = d.run_id
                  WHERE d.from_principal = ?1 AND d.state = ?2 AND r.status = 'active'",
                params![principal, STATE_REQUESTED],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        if left > 0 {
            tracing::error!(
                "revocation drain INCOMPLETE for {principal}: {left} active run(s) remain                  beyond the {DRAIN_MAX_PAGES}-page budget — re-run the revoke or drain manually"
            );
            crate::audit::record(
                conn,
                crate::audit::AuditKind::Auth,
                revoked_by,
                &format!("principal:{principal}"),
                crate::audit::AuditStatus::Error,
                &format!("drain_incomplete: {left} active run(s) remain"),
            );
        }
    }
    Ok(drained)
}

/// Runs a revoked principal leaves WEDGED as a delegatee: active runs with
/// in-flight (`requested`) delegations OWED to it (`to_principal`). The
/// drain cancels work the principal OWNS (`from_principal`); work it owes
/// stays `active` with an uncompletable delegation (the result path
/// re-checks revocation and refuses forever), so the operator must cancel
/// those runs by hand. This query surfaces them — the revoke response
/// carries the ids (bounded, oldest first) instead of leaving the operator
/// to discover the wedge.
pub(crate) fn wedged_delegations(
    conn: &Connection,
    principal: &str,
) -> Result<Vec<i64>, MeshError> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT d.run_id FROM delegations d
               JOIN workflow_runs r ON r.id = d.run_id
              WHERE d.to_principal = ?1 AND d.state = ?2 AND r.status = 'active'
              ORDER BY d.run_id LIMIT 500",
        )
        .map_err(|e| MeshError::Database(e.to_string()))?;
    stmt.query_map(params![principal, STATE_REQUESTED], |r| r.get(0))
        .map_err(|e| MeshError::Database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| MeshError::Database(e.to_string()))
}

/// The revocation register for the operator surface (bounded, newest first).
pub(crate) fn list_revocations(conn: &Connection) -> Result<Vec<serde_json::Value>, MeshError> {
    conn.prepare(
        "SELECT principal, revoked_at, reason, revoked_by
           FROM revoked_principals ORDER BY revoked_at DESC, principal LIMIT 500",
    )
    .map_err(|e| MeshError::Database(e.to_string()))?
    .query_map([], |r| {
        Ok(serde_json::json!({
            "principal": r.get::<_, String>(0)?,
            "revoked_at": r.get::<_, i64>(1)?,
            "reason": r.get::<_, String>(2)?,
            "revoked_by": r.get::<_, String>(3)?,
        }))
    })
    .map_err(|e| MeshError::Database(e.to_string()))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| MeshError::Database(e.to_string()))
}

/// The provisioning draft. `capabilities_json` must be a JSON object — it is
/// the A2A `capabilities`/`skills` block, validated at the boundary.
#[derive(Debug)]
pub struct CardDraft<'a> {
    pub domain: &'a str,
    pub principal: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub capabilities_json: &'a str,
}

/// One verified card as emitted to callers (signature fields included so a
/// consumer can re-verify independently).
#[derive(Debug, Clone)]
pub struct AgentCard {
    pub id: i64,
    pub domain: String,
    pub principal: String,
    pub name: String,
    pub description: String,
    pub capabilities_json: String,
    pub card_json: String,
    pub signature_hex: String,
    pub signed_by: String,
    /// The operator-key generation that signed this card (additive; legacy
    /// rows are NULL = verify against current-or-previous, the old
    /// binaries' behavior plus the overlap window).
    pub signing_epoch: Option<i64>,
}

fn validate_card(draft: &CardDraft) -> Result<(), MeshError> {
    if draft.principal.is_empty() || draft.principal.len() > MAX_PRINCIPAL_LEN {
        return Err(MeshError::InvalidInput("principal", "1..=256 chars"));
    }
    if draft.name.trim().is_empty() || draft.name.len() > MAX_NAME_LEN {
        return Err(MeshError::InvalidInput("name", "1..=200 chars"));
    }
    if draft.description.len() > MAX_DESCRIPTION_LEN {
        return Err(MeshError::InvalidInput("description", "too long"));
    }
    let caps: serde_json::Value = serde_json::from_str(draft.capabilities_json)
        .map_err(|_| MeshError::InvalidInput("capabilities", "must be a JSON object"))?;
    if !caps.is_object() || draft.capabilities_json.len() > MAX_CAPABILITIES_LEN {
        return Err(MeshError::InvalidInput(
            "capabilities",
            "must be an object ≤4000 bytes",
        ));
    }
    Ok(())
}

/// The canonical A2A-shaped manifest string that gets signed. Field order is
/// fixed by struct declaration order — the exact bytes are what the signature
/// covers, and the same bytes are what reads return.
fn card_document(draft: &CardDraft) -> String {
    serde_json::to_string(&serde_json::json!({
        "type": "agent-card",
        "protocol_version": "0.3",
        "name": draft.name,
        "description": draft.description,
        "principal": draft.principal,
        "domain": draft.domain,
        "capabilities": draft.capabilities_json,
    }))
    .unwrap_or_default()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Insert (or replace) one agent's signed card. Signing happens HERE, at
/// provisioning, with the UMP operator key; no key ⇒ loud refusal. The caller
/// owns the surrounding transaction.
pub(crate) fn provision_card(
    conn: &Connection,
    draft: &CardDraft,
    now: i64,
) -> Result<AgentCard, MeshError> {
    validate_card(draft)?;
    let (_, sk) = crate::handlers::ump::operator_signing_key().ok_or(MeshError::NoOperatorKey)?;
    let card_json = card_document(draft);
    let sig = ed25519_dalek::Signer::sign(&sk, sha256_hex(card_json.as_bytes()).as_bytes());
    let signature_hex = hex::encode(sig.to_bytes());
    let signed_by = crate::handlers::ump::did_key(&sk.verifying_key().to_bytes());
    // The card's signing epoch = the operator-key generation at signing
    // time. `verify_card` uses it to pick the key deterministically; the
    // audit trail names the generation.
    let epoch = operator_key_generation(conn);
    conn.execute(
        "INSERT INTO agent_cards(domain, principal, name, description, capabilities_json,
             card_json, signature, signed_by, signing_epoch, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(domain, principal) DO UPDATE SET
             name = excluded.name, description = excluded.description,
             capabilities_json = excluded.capabilities_json, card_json = excluded.card_json,
             signature = excluded.signature, signed_by = excluded.signed_by,
             signing_epoch = excluded.signing_epoch,
             created_at = excluded.created_at",
        params![
            draft.domain,
            draft.principal,
            draft.name,
            draft.description,
            draft.capabilities_json,
            card_json,
            signature_hex,
            signed_by,
            epoch,
            now
        ],
    )
    .map_err(|e| MeshError::Database(e.to_string()))?;
    Ok(AgentCard {
        id: conn
            .query_row(
                "SELECT id FROM agent_cards WHERE domain = ?1 AND principal = ?2",
                params![draft.domain, draft.principal],
                |r| r.get(0),
            )
            .map_err(|e| MeshError::Database(e.to_string()))?,
        domain: draft.domain.to_string(),
        principal: draft.principal.to_string(),
        name: draft.name.to_string(),
        description: draft.description.to_string(),
        capabilities_json: draft.capabilities_json.to_string(),
        card_json,
        signature_hex,
        signed_by,
        signing_epoch: Some(epoch),
    })
}

/// Read ONE card and VERIFY its signature against the CURRENT operator key.
/// This is the "verified at token use" law: every use of a card — serving it
/// or accepting a delegation naming it — re-checks the chain to the key. A
/// tampered `card_json`/`signature` pair or a rotated-away key denies.
pub(crate) fn verify_card(
    conn: &Connection,
    domain: &str,
    principal: &str,
) -> Result<AgentCard, MeshError> {
    // Revocation FIRST (ASI03/07): a revoked principal's card fails closed
    // before any signature work — and before the row lookup, so revocation
    // stays probe-blind (no card-existence oracle for a revoked identity).
    // The signature check that follows is fail-closed either way; the order
    // only decides WHICH loud refusal a revoked principal sees.
    if is_revoked(conn, principal)? {
        return Err(MeshError::PrincipalRevoked(principal.to_string()));
    }
    let row = conn
        .query_row(
            "SELECT id, principal, name, description, capabilities_json, card_json,
                    signature, signed_by, domain, signing_epoch
               FROM agent_cards WHERE domain = ?1 AND principal = ?2",
            params![domain, principal],
            |r| {
                Ok(AgentCard {
                    id: r.get(0)?,
                    principal: r.get(1)?,
                    name: r.get(2)?,
                    description: r.get(3)?,
                    capabilities_json: r.get(4)?,
                    card_json: r.get(5)?,
                    signature_hex: r.get(6)?,
                    signed_by: r.get(7)?,
                    domain: r.get(8)?,
                    signing_epoch: r.get(9)?,
                })
            },
        )
        .optional()
        .map_err(|e| MeshError::Database(e.to_string()))?
        .ok_or_else(|| MeshError::CardUnknown(principal.to_string()))?;
    // The overlap window: cards signed by the PREVIOUS generation keep
    // verifying through `operator.ed25519.prev`; the current generation
    // verifies against the current key. Signing is ALWAYS the current key
    // (see sign_card); this is the verify-only seam the rotation ceremony
    // relies on. Legacy rows (NULL epoch — pre-column) try both keys, which
    // is exactly the old binaries' behavior plus the window.
    let keys = crate::handlers::ump::operator_verify_keys();
    if keys.is_empty() {
        return Err(MeshError::NoOperatorKey);
    }
    let current_gen = operator_key_generation(conn);
    let chosen: Vec<&ed25519_dalek::SigningKey> = match row.signing_epoch {
        Some(epoch) if epoch == current_gen => vec![&keys[0]],
        Some(epoch) if epoch == current_gen - 1 && keys.len() > 1 => vec![&keys[1]],
        // Legacy / unknown epoch: current first, then the overlap window.
        _ => keys.iter().collect(),
    };
    let sig_bytes: [u8; 64] = hex::decode(&row.signature_hex)
        .ok()
        .and_then(|v| <[u8; 64]>::try_from(v).ok())
        .ok_or_else(|| MeshError::CardTampered(principal.to_string()))?;
    let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
    let msg = sha256_hex(row.card_json.as_bytes());
    let verified = chosen.iter().any(|sk| {
        sk.verifying_key()
            .verify_strict(msg.as_bytes(), &sig)
            .is_ok()
    });
    if !verified {
        return Err(MeshError::CardTampered(principal.to_string()));
    }
    Ok(row)
}

/// All cards in a domain, each verified before it may leave the server.
pub(crate) fn list_cards(conn: &Connection, domain: &str) -> Result<Vec<AgentCard>, MeshError> {
    let principals: Vec<String> = conn
        .prepare("SELECT principal FROM agent_cards WHERE domain = ?1 ORDER BY principal")?
        .query_map(params![domain], |r| r.get(0))?
        .collect::<Result<_, _>>()
        .map_err(|e| MeshError::Database(e.to_string()))?;
    principals
        .iter()
        .map(|p| verify_card(conn, domain, p))
        .collect()
}

/// The pure working-set arbiter: (base domain, agent principal) → the agent's
/// own scratch-domain name. Deterministic, charset-legal (same law as
/// [`crate::storage_layout::is_valid_domain`]), collision-safe via a
/// content hash of the principal. Agent writes land here; the shared base
/// domain only receives promoted knowledge through the proposal gate.
pub fn working_set_domain(base_domain: &str, agent_principal: &str) -> String {
    let digest = sha256_hex(agent_principal.as_bytes());
    let stem: String = base_domain
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
        .take(40)
        .collect();
    format!("{stem}-ws-{}", &digest[..12])
}

/// True when `domain` was produced by [`working_set_domain`] for this base —
/// the read-side marker that lets surfaces tell agent scratch from shared
/// knowledge. Pure suffix/shape check over the same derivation.
pub fn is_working_set_domain(base_domain: &str, candidate: &str) -> bool {
    let stem: String = base_domain
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
        .take(40)
        .collect();
    candidate
        .strip_prefix(&format!("{stem}-ws-"))
        .map(|rest| rest.len() == 12 && rest.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or(false)
}

/// One delegation: a named agent's work order over a run.
#[derive(Debug)]
pub struct DelegationDraft<'a> {
    pub domain: &'a str,
    pub run_id: i64,
    pub from_principal: &'a str,
    pub to_principal: &'a str,
    pub screened_task: &'a str,
    pub key_suffix: &'a str,
    pub now: i64,
}

#[derive(Debug)]
pub struct DelegationOutcome {
    pub delegation_id: i64,
    pub event_id: i64,
    pub card: AgentCard,
}

/// Enqueue an agent→agent task: verify the target's card FIRST (an unverified
/// or unknown agent cannot be delegated to — fail closed), then insert the
/// row + lineage event + audit INSIDE the caller's tx. Task content stays in
/// the table; the lineage payload carries ids + actors only.
pub(crate) fn request_delegation(
    conn: &Connection,
    draft: &DelegationDraft,
) -> Result<DelegationOutcome, MeshError> {
    // No new work dispatch post-revocation: a revoked DISPATCHER refuses
    // before anything is written; a revoked TARGET refuses via verify_card's
    // pre-signature revocation check below.
    if is_revoked(conn, draft.from_principal)? {
        return Err(MeshError::PrincipalRevoked(
            draft.from_principal.to_string(),
        ));
    }
    let card = verify_card(conn, draft.domain, draft.to_principal)?;
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM delegations WHERE run_id = ?1",
            params![draft.run_id],
            |r| r.get(0),
        )
        .map_err(|e| MeshError::Database(e.to_string()))?;
    if n >= MAX_DELEGATIONS_PER_RUN {
        return Err(MeshError::DelegationsFull);
    }
    conn.execute(
        "INSERT INTO delegations(domain, run_id, from_principal, to_principal, task, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            draft.domain,
            draft.run_id,
            draft.from_principal,
            draft.to_principal,
            draft.screened_task,
            STATE_REQUESTED,
            draft.now
        ],
    )
    .map_err(|e| MeshError::Database(e.to_string()))?;
    let delegation_id = conn.last_insert_rowid();
    let event_id = super::outbox::append_lineage(
        conn,
        draft.run_id,
        TOPIC_REQUEST,
        &serde_json::json!({
            "action": "request",
            "delegation_id": delegation_id,
            "to": draft.to_principal,
            "from": draft.from_principal,
            "card_name": card.name,
        })
        .to_string(),
        &format!("del:{}:{delegation_id}", draft.key_suffix),
        draft.now,
    )
    .map_err(|e| MeshError::Database(e.to_string()))?;
    super::audit_write(
        conn,
        draft.run_id,
        &format!("delegation:{delegation_id}"),
        AuditStatus::Ok,
        "delegation:request",
    );
    Ok(DelegationOutcome {
        delegation_id,
        event_id,
        card,
    })
}

/// Submit the delegated work's result: ONLY the delegated agent, exactly once
/// (CAS `requested → completed`), as a child lineage event at the current tip.
pub(crate) fn submit_result(
    conn: &Connection,
    run_id: i64,
    delegation_id: i64,
    actor: &str,
    screened_result: &str,
    now: i64,
) -> Result<i64, MeshError> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT to_principal, state FROM delegations WHERE id = ?1 AND run_id = ?2",
            params![delegation_id, run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| MeshError::Database(e.to_string()))?;
    let (to, state) = row.ok_or(MeshError::NotFound("delegation"))?;
    if actor != to {
        return Err(MeshError::NotDelegatee(actor.to_string()));
    }
    // Decision-time re-check (ASI03/07): a delegation accepted before its
    // agent was revoked can never return a result afterwards. The row
    // persists (evidence), the verdict is always refusal.
    if is_revoked(conn, actor)? {
        return Err(MeshError::PrincipalRevoked(actor.to_string()));
    }
    if state != STATE_REQUESTED {
        return Err(MeshError::AlreadyCompleted);
    }
    let changed = conn
        .execute(
            "UPDATE delegations SET result = ?1, state = 'completed', decided_at = ?2
              WHERE id = ?3 AND state = 'requested'",
            params![screened_result, now, delegation_id],
        )
        .map_err(|e| MeshError::Database(e.to_string()))?;
    if changed != 1 {
        return Err(MeshError::AlreadyCompleted);
    }
    let event_id = super::outbox::append_lineage(
        conn,
        run_id,
        TOPIC_RESULT,
        &serde_json::json!({
            "action": "result",
            "delegation_id": delegation_id,
            "by": actor,
        })
        .to_string(),
        &format!("del-res:{delegation_id}"),
        now,
    )
    .map_err(|e| MeshError::Database(e.to_string()))?;
    super::audit_write(
        conn,
        run_id,
        &format!("delegation:{delegation_id}"),
        AuditStatus::Ok,
        "delegation:result",
    );
    Ok(event_id)
}

#[derive(Debug)]
pub struct DelegationRow {
    pub id: i64,
    pub from_principal: String,
    pub to_principal: String,
    pub task: String,
    pub state: String,
    pub result: Option<String>,
    pub created_at: i64,
    pub decided_at: Option<i64>,
}

/// The delegation view for one run, chronological, bounded.
pub(crate) fn list_delegations(
    conn: &Connection,
    run_id: i64,
    offset: i64,
    limit: i64,
) -> Result<Vec<DelegationRow>, MeshError> {
    conn.prepare(
        "SELECT id, from_principal, to_principal, task, state, result, created_at, decided_at
           FROM delegations WHERE run_id = ?1 ORDER BY id LIMIT ?2 OFFSET ?3",
    )?
    .query_map(params![run_id, limit.clamp(0, 200), offset.max(0)], |r| {
        Ok(DelegationRow {
            id: r.get(0)?,
            from_principal: r.get(1)?,
            to_principal: r.get(2)?,
            task: r.get(3)?,
            state: r.get(4)?,
            result: r.get(5)?,
            created_at: r.get(6)?,
            decided_at: r.get(7)?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| MeshError::Database(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::tx::WorkflowTx;
    use rusqlite::Connection;
    // Env-var config is process-global: every test that points
    // `BRAIN_UMP_KEY_DIR` at a temp seed takes THE shared lock — per-module
    // locks guarded nothing across modules (the standby proptest flaked on
    // CI exactly that way).
    use crate::test_support::lock_env;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 'active', 1000, 1000)",
            [],
        )
        .unwrap();
        conn
    }

    /// Point the operator key at a temp dir holding a fresh 0600 seed; the
    /// guard restores the previous env on drop.
    struct OperatorKey(tempfile::TempDir);
    impl OperatorKey {
        fn new() -> OperatorKey {
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::write(dir.path().join("operator.key"), [7u8; 32]).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    dir.path().join("operator.key"),
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            // SAFETY: single-threaded under ENV_LOCK — the documented env-mutation posture.
            unsafe { std::env::set_var("BRAIN_UMP_KEY_DIR", dir.path()) };
            OperatorKey(dir)
        }
    }
    impl Drop for OperatorKey {
        fn drop(&mut self) {
            // SAFETY: single-threaded under ENV_LOCK.
            unsafe { std::env::remove_var("BRAIN_UMP_KEY_DIR") };
        }
    }

    /// Simulate one rotation the way `brain key rotate` does: rename the
    /// current key file to `.prev`, write a fresh seed at the fixed name,
    /// bump the generation counter in the DB.
    fn rotate(dir: &std::path::Path, conn: &Connection) {
        std::fs::rename(
            dir.join(crate::handlers::ump::OPERATOR_KEY_FILE),
            dir.join(crate::handlers::ump::OPERATOR_KEY_PREV_FILE),
        )
        .unwrap();
        let seed: Vec<u8> = (0..32).map(|i| (i * 31 + 7) as u8).collect();
        std::fs::write(dir.join(crate::handlers::ump::OPERATOR_KEY_FILE), &seed).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                dir.join(crate::handlers::ump::OPERATOR_KEY_FILE),
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO schema_meta(key, value) VALUES ('operator_key_generation', '1')
             ON CONFLICT(key) DO UPDATE SET
                 value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
            [],
        )
        .unwrap();
    }

    /// The rotation window: a card signed before `brain key rotate` keeps
    /// verifying through `operator.ed25519.prev`; a card provisioned after
    /// signs with (and verifies against) the CURRENT key only.
    #[test]
    fn rotate_keeps_old_card_verifying_via_prev() {
        let _guard = lock_env();
        let key = OperatorKey::new();
        let conn = db();
        let old = provision_card(&conn, &card("atlas"), 1100).expect("provisioned");
        verify_card(&conn, "acme", "atlas").expect("pre-rotation verify");

        rotate(key.0.path(), &conn);

        // The old card still verifies (the ONE-deep overlap window).
        verify_card(&conn, "acme", "atlas").expect("post-rotation verify via prev");
        // A NEW card signs with the CURRENT key and records the new epoch.
        let fresh = provision_card(&conn, &card("hermes"), 1200).expect("re-provisioned");
        assert_ne!(
            old.signed_by, fresh.signed_by,
            "signing ALWAYS uses the current key"
        );
        verify_card(&conn, "acme", "hermes").expect("new card verifies via current");
    }

    /// The window is ONE key deep: after a SECOND rotate, the first
    /// generation's cards die (the .prev slot holds the middle generation).
    #[test]
    fn third_generation_kills_first() {
        let _guard = lock_env();
        let key = OperatorKey::new();
        let conn = db();
        let first = provision_card(&conn, &card("gen1"), 1100).expect("gen1 card");

        rotate(key.0.path(), &conn); // gen 1
        rotate(key.0.path(), &conn); // gen 2

        // gen-1's signer is GONE (prev now holds gen-2's predecessor = gen-1's
        // key? no: prev holds the key current BEFORE the second rotate = the
        // MIDDLE key). The FIRST generation cannot verify.
        assert!(
            matches!(
                verify_card(&conn, "acme", "gen1"),
                Err(MeshError::CardTampered(_))
            ),
            "the one-deep window means a second rotate orphans the first generation (pinned)"
        );
        assert_eq!(first.signing_epoch, Some(0));
    }

    /// Every provisioned card records its signing epoch — the audit trail
    /// names the generation and verify picks the key deterministically.
    #[test]
    fn card_epoch_recorded() {
        let _guard = lock_env();
        let key = OperatorKey::new();
        let conn = db();
        let c = provision_card(&conn, &card("atlas"), 1100).expect("provisioned");
        assert_eq!(c.signing_epoch, Some(0));
        rotate(key.0.path(), &conn);
        let c2 = provision_card(&conn, &card("atlas2"), 1200).expect("provisioned");
        assert_eq!(c2.signing_epoch, Some(1));
    }

    fn card(principal: &str) -> CardDraft<'_> {
        CardDraft {
            domain: "acme",
            principal,
            name: "Atlas",
            description: "network diagnostics agent",
            capabilities_json: r#"{"skills":["networking"]}"#,
        }
    }

    /// agent_card_signature_verified_on_principal_use — a provisioned card
    /// verifies at every use point; a tampered manifest or signature refuses
    /// loudly (fail closed), and an unknown agent refuses too.
    #[test]
    fn agent_card_signature_verified_on_principal_use() {
        let _guard = lock_env();
        let _key = OperatorKey::new();
        let conn = db();

        let stored = provision_card(&conn, &card("atlas"), 1100).expect("provisioned");
        assert!(!stored.signature_hex.is_empty());
        assert!(stored.signed_by.starts_with("did:key:"));

        // Use point: verification passes on the honest card.
        verify_card(&conn, "acme", "atlas").expect("verified");

        // Tamper with the manifest: the stored bytes no longer match the sig.
        conn.execute(
            "UPDATE agent_cards SET card_json = replace(card_json, 'Atlas', 'Malice'),
                   name = 'Malice' WHERE principal = 'atlas'",
            [],
        )
        .unwrap();
        assert!(
            matches!(
                verify_card(&conn, "acme", "atlas"),
                Err(MeshError::CardTampered(_))
            ),
            "a tampered card must refuse loudly"
        );

        // Re-provision repairs the card (operator re-signs); then flip ONLY the
        // signature hex — same refusal.
        provision_card(&conn, &card("atlas"), 1200).unwrap();
        verify_card(&conn, "acme", "atlas").unwrap();
        conn.execute(
            "UPDATE agent_cards SET signature = ?1 || substr(signature, 3)
              WHERE principal = 'atlas'",
            params!["00"],
        )
        .unwrap();
        assert!(matches!(
            verify_card(&conn, "acme", "atlas"),
            Err(MeshError::CardTampered(_))
        ));

        // Unknown agent: no card, no delegation target.
        assert!(matches!(
            verify_card(&conn, "acme", "ghost"),
            Err(MeshError::CardUnknown(_))
        ));
        // list_cards fails CLOSED on a tampered card — no partial roster.
        assert!(matches!(
            list_cards(&conn, "acme"),
            Err(MeshError::CardTampered(_))
        ));
        provision_card(&conn, &card("atlas"), 1300).unwrap();
        assert_eq!(list_cards(&conn, "acme").unwrap().len(), 1);
    }

    /// delegation_request_and_result_are_lineage_events — a request verifies
    /// the target's card first, appends a `delegation/request` lineage event
    /// whose payload carries ids + actors (never task content), audits in-tx;
    /// the result is the delegatee-only CAS completion with its own child
    /// event; a replayed result refuses.
    #[test]
    fn delegation_request_and_result_are_lineage_events() {
        let _guard = lock_env();
        let _key = OperatorKey::new();
        let mut conn = db();

        provision_card(&conn, &card("atlas"), 1000).unwrap();

        let draft = DelegationDraft {
            domain: "acme",
            run_id: 1,
            from_principal: "human",
            to_principal: "atlas",
            screened_task: "check the router logs",
            key_suffix: "k1",
            now: 1100,
        };
        {
            let mut tx = WorkflowTx::begin(&mut conn).unwrap();
            let out = request_delegation(tx.tx(), &draft).expect("delegated");
            tx.commit().unwrap();
            assert_eq!(out.card.principal, "atlas");
        }

        // The lineage event exists, parented at the tip, content-free.
        let (topic, payload): (String, String) = conn
            .query_row(
                "SELECT topic, payload_json FROM outbox WHERE idempotency_key = 'del:k1:1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(topic, TOPIC_REQUEST);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["delegation_id"], 1);
        assert_eq!(v["to"], "atlas");
        assert!(
            !payload.contains("router logs"),
            "task content never rides the lineage payload"
        );
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events
                  WHERE kind='workflow' AND target_hash = ?1",
                params![crate::audit::hash("delegation:1")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the request audited inside the transition");

        // Result: only atlas may submit; exactly once.
        {
            let mut tx = WorkflowTx::begin(&mut conn).unwrap();
            submit_result(tx.tx(), 1, 1, "bob", "staged result", 1200).unwrap_err();
            submit_result(tx.tx(), 1, 1, "atlas", "logs show DHCP exhaustion", 1200)
                .expect("result accepted");
            tx.commit().unwrap();
        }
        assert!(matches!(
            submit_result(&conn, 1, 1, "atlas", "again", 1300),
            Err(MeshError::AlreadyCompleted)
        ));
        let (state, result): (String, String) = conn
            .query_row(
                "SELECT state, result FROM delegations WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, STATE_COMPLETED);
        assert_eq!(result, "logs show DHCP exhaustion");
        let result_topic: String = conn
            .query_row(
                "SELECT topic FROM outbox WHERE idempotency_key = 'del-res:1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(result_topic, TOPIC_RESULT);

        // Unverified delegation target refuses BEFORE any row is written.
        let bad = DelegationDraft {
            to_principal: "ghost",
            key_suffix: "k2",
            ..draft
        };
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM delegations", [], |r| r.get(0))
            .unwrap();
        assert!(matches!(
            request_delegation(&conn, &bad),
            Err(MeshError::CardUnknown(_))
        ));
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM delegations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after, "an unverified target writes nothing");

        let view = list_delegations(&conn, 1, 0, 200).unwrap();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].to_principal, "atlas");
    }

    /// agent_working_set_isolated_until_promoted — the pure arbiter maps
    /// (base, agent) to a charset-legal scratch domain distinct from the base,
    /// deterministic across calls and agents, and `is_working_set_domain`
    /// separates scratch from shared so promotion stays the proposal gate's job.
    #[test]
    fn agent_working_set_isolated_until_promoted() {
        let ws = working_set_domain("global", "atlas");
        assert_ne!(ws, "global", "the working set is never the shared domain");
        assert_eq!(ws, working_set_domain("global", "atlas"));
        assert_ne!(
            working_set_domain("global", "atlas"),
            working_set_domain("global", "orion"),
            "each agent namespaces its own scratch"
        );
        // Charset law holds (same rules storage_layout enforces for filenames).
        assert!(ws.len() <= 63 && !ws.is_empty());
        assert!(
            ws.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        );
        assert!(is_working_set_domain("global", &ws));
        assert!(
            !is_working_set_domain("global", "global"),
            "the shared domain never reads as an agent's scratch"
        );
        assert!(!is_working_set_domain("other", &ws));
    }

    /// revoked_principal_cards_fail_closed — the ASI03/07 kill-switch read
    /// side: after revocation, the card refuses BEFORE signature work
    /// (probe-blind: the refusal is `PrincipalRevoked`, not
    /// `CardUnknown`/`CardTampered`), list refuses closed, a dispatch to the
    /// revoked agent refuses, and a result the revoked agent owes refuses at
    /// decision time even though the delegation was accepted pre-revocation.
    #[test]
    fn revoked_principal_cards_fail_closed() {
        let _guard = lock_env();
        let _key = OperatorKey::new();
        let mut conn = db();

        provision_card(&conn, &card("atlas"), 1000).unwrap();
        verify_card(&conn, "acme", "atlas").expect("verified pre-revocation");

        // Dispatch a delegation to atlas, then revoke atlas.
        let draft = DelegationDraft {
            domain: "acme",
            run_id: 1,
            from_principal: "human",
            to_principal: "atlas",
            screened_task: "check the router logs",
            key_suffix: "k1",
            now: 1100,
        };
        {
            let mut tx = WorkflowTx::begin(&mut conn).unwrap();
            request_delegation(tx.tx(), &draft).expect("delegated pre-revocation");
            tx.commit().unwrap();
        }

        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        let drained = revoke_principal(tx.tx(), "atlas", "compromised agent", "operator", 1200)
            .expect("revoked");
        tx.commit().unwrap();
        assert_eq!(
            drained, 0,
            "atlas OWNS no in-flight work here (it owes some)"
        );
        // A5-06: what atlas OWES is surfaced, not drained — the run stays
        // active with an uncompletable delegation, and the operator needs
        // the id to cancel it by hand.
        let wedged = wedged_delegations(&conn, "atlas").expect("wedge query reads");
        assert_eq!(
            wedged,
            vec![1],
            "the delegatee-side wedge names the run the revoked principal owes"
        );
        let wedged_human = wedged_delegations(&conn, "human").expect("wedge query reads");
        assert!(
            wedged_human.is_empty(),
            "the owner side is drained, not wedged"
        );

        // Card use: revoked, BEFORE any signature work — and probe-blind
        // (revoked wins even though the card row still exists).
        assert!(matches!(
            verify_card(&conn, "acme", "atlas"),
            Err(MeshError::PrincipalRevoked(_))
        ));
        // list_cards fails CLOSED on a revoked card — no partial roster.
        assert!(matches!(
            list_cards(&conn, "acme"),
            Err(MeshError::PrincipalRevoked(_))
        ));
        // A revoked agent cannot return its result (decision-time re-check).
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        let err = submit_result(tx.tx(), 1, 1, "atlas", "the result", 1300).unwrap_err();
        tx.commit().unwrap();
        assert!(
            matches!(err, MeshError::PrincipalRevoked(_)),
            "result after revocation must refuse closed, got {err:?}"
        );

        // Re-provisioning the card does NOT resurrect the identity: the
        // revocation row outlives any re-signed manifest.
        provision_card(&conn, &card("atlas"), 1400).unwrap();
        assert!(matches!(
            verify_card(&conn, "acme", "atlas"),
            Err(MeshError::PrincipalRevoked(_))
        ));

        // The audit chain carries the revocation (kind auth, target principal).
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events
                  WHERE kind='auth' AND target_hash = ?1 AND detail_hash = ?2",
                params![
                    crate::audit::hash("principal:atlas"),
                    crate::audit::hash("revoke:compromised agent")
                ],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the revocation is hash-chained evidence");
    }

    /// revoked_owner_no_new_dispatch — the drain: revoking a principal
    /// cancels every ACTIVE run where they own in-flight (`requested`)
    /// delegation work through the EXISTING cancel path (status
    /// `cancelled` via the run CAS), leaves the lineage marker, and any
    /// further dispatch BY the revoked owner refuses before anything is
    /// written. A COMPLETED owner-delegation does not cancel the run (the
    /// run has no in-flight work of theirs left to drain).
    #[test]
    fn revoked_owner_no_new_dispatch() {
        let _guard = lock_env();
        let _key = OperatorKey::new();
        let mut conn = db();

        provision_card(&conn, &card("atlas"), 1000).unwrap();
        // Run 1 is active (fixture); the owner `human` holds in-flight work.
        let draft = DelegationDraft {
            domain: "acme",
            run_id: 1,
            from_principal: "human",
            to_principal: "atlas",
            screened_task: "check the router logs",
            key_suffix: "k1",
            now: 1100,
        };
        {
            let mut tx = WorkflowTx::begin(&mut conn).unwrap();
            request_delegation(tx.tx(), &draft).expect("delegated");
            tx.commit().unwrap();
        }

        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        let drained =
            revoke_principal(tx.tx(), "human", "offboarded operator", "dpo", 1200).expect("ok");
        tx.commit().unwrap();
        assert_eq!(drained, 1, "the owner's one in-flight run drains");

        // The EXISTING cancel path did the write: status is `cancelled`,
        // state_revision advanced by the CAS, state_json untouched.
        let (status, revision): (String, i64) = conn
            .query_row(
                "SELECT status, state_revision FROM workflow_runs WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, STATE_CANCELLED);
        assert_eq!(revision, 1, "the CAS advanced exactly once");

        // The drain is observed in events (the drill's evidence read).
        let (topic, payload): (String, String) = conn
            .query_row(
                "SELECT topic, payload_json FROM outbox
                  WHERE idempotency_key = 'revoked:human:1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(topic, TOPIC_REVOKED);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["action"], "revocation_drain");
        assert_eq!(v["principal"], "human");

        // No NEW dispatch post-revocation: the revoked owner refuses before
        // any row is written.
        let draft2 = DelegationDraft {
            key_suffix: "k2",
            now: 1300,
            ..draft
        };
        {
            let mut tx = WorkflowTx::begin(&mut conn).unwrap();
            let err = request_delegation(tx.tx(), &draft2).unwrap_err();
            tx.commit().unwrap();
            assert!(matches!(err, MeshError::PrincipalRevoked(_)));
        }
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM delegations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "no new delegation row post-revocation");

        // And the run-scoped audit row marks the drain.
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events
                  WHERE kind='workflow' AND detail_hash = ?1",
                params![crate::audit::hash("revocation drain (owner human)")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }
}
