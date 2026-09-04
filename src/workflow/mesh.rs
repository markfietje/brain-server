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

pub const STATE_REQUESTED: &str = "requested";
pub const STATE_COMPLETED: &str = "completed";

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
            MeshError::Database(m) => write!(f, "{m}"),
        }
    }
}

impl From<rusqlite::Error> for MeshError {
    fn from(e: rusqlite::Error) -> Self {
        MeshError::Database(e.to_string())
    }
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
    conn.execute(
        "INSERT INTO agent_cards(domain, principal, name, description, capabilities_json,
             card_json, signature, signed_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(domain, principal) DO UPDATE SET
             name = excluded.name, description = excluded.description,
             capabilities_json = excluded.capabilities_json, card_json = excluded.card_json,
             signature = excluded.signature, signed_by = excluded.signed_by,
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
    let row = conn
        .query_row(
            "SELECT id, principal, name, description, capabilities_json, card_json,
                    signature, signed_by, domain
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
                })
            },
        )
        .optional()
        .map_err(|e| MeshError::Database(e.to_string()))?
        .ok_or_else(|| MeshError::CardUnknown(principal.to_string()))?;
    let (_, sk) = crate::handlers::ump::operator_signing_key().ok_or(MeshError::NoOperatorKey)?;
    let sig_bytes: [u8; 64] = hex::decode(&row.signature_hex)
        .ok()
        .and_then(|v| <[u8; 64]>::try_from(v).ok())
        .ok_or_else(|| MeshError::CardTampered(principal.to_string()))?;
    let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
    sk.verifying_key()
        .verify_strict(sha256_hex(row.card_json.as_bytes()).as_bytes(), &sig)
        .map_err(|_| MeshError::CardTampered(principal.to_string()))?;
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
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// Env-var config is process-global: every test that points
    /// `BRAIN_UMP_KEY_DIR` at a temp seed takes the shared lock, tolerantly.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
    }

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

    /// cross_agent_recall_shows_origin_labels — knowledge written by an agent
    /// carries `origin='agent'` through the read-seam shaping, so any other
    /// principal's recall hit shows provenance instead of anonymous content.
    #[test]
    fn cross_agent_recall_shows_origin_labels() {
        let peer: Option<crate::auth::Principal> = None;
        // The exact shaping the recall hit builder applies to stored origin
        // text (handlers/recall.rs): provenance labels pass the read seam
        // intact for any viewer, agent-authored or not.
        assert_eq!(
            crate::gate::sanitize_read_opt(Some("agent".into()), false, &peer),
            Some("agent".into())
        );
        assert_eq!(
            crate::gate::sanitize_read_opt(Some("agent:atlas".into()), false, &peer),
            Some("agent:atlas".into())
        );
        // And an agent-authored hit's label survives PII-mode shaping too.
        assert_eq!(
            crate::gate::sanitize_read_opt(Some("agent".into()), true, &peer),
            Some("agent".into())
        );
    }
}
