//! The agent-session event log — the Loop line's session STORE.
//!
//! The harness's pending-write queue drains into the outbox; THIS table is
//! the replayable per-run session narrative (user turns, assistant turns,
//! tool results, compaction summaries) the loop rebuilds context from. It is
//! append-only and prefix-stable: rows are never mutated or reordered, seq
//! is per-run monotonic, and exactly-once holds by idempotency key — a
//! replayed append returns the original receipt, never a second row.
//!
//! Why a table and not the audit chain: `audit_events` stores hashes of
//! targets/details (tamper evidence), not payloads — a log you cannot read
//! back cannot be replayed. Every append still emits its [`AuditKind::Workflow`]
//! row inside the caller's transaction via the substrate's [`audit_write`],
//! so the chain can reconstruct THAT a session event landed, while this
//! table carries the WHAT.
//!
//! Like the rest of the substrate: SQL lives here, callers pass
//! `&Connection` (a `WorkflowTx` for multi-write atomicity), time enters as
//! an argument, foreign runs are refused by the writer's query not by schema
//! (the outbox posture — no FK on purpose).

use rusqlite::{Connection, OptionalExtension, params};

use super::audit_write;
use crate::audit::AuditStatus;

/// Default replay window (bounds law): the loop asks for at most this many
/// most-recent events; pinned so no caller can ask for "all of history"
/// without visibly raising a number.
pub(crate) const REPLAY_CAP: usize = 500;

/// Payload bound (bounds law): an event larger than this is refused, not
/// truncated — a loop that tries to persist a 10-MiB tool blob fails loud
/// and the caller decides policy. Matches the hostcall effect-output cap.
pub(crate) const PAYLOAD_CAP_BYTES: usize = 64 * 1024;

/// One replayed session event, in seq order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionEventRow {
    pub seq: i64,
    pub kind: String,
    pub payload_json: String,
    pub created_at: i64,
}

/// Append one event to the run's session log. Returns `(created, seq)`:
/// a first append created its row and drew the next per-run seq; a replay
/// of a known idempotency key is a no-op receipt returning the ORIGINAL seq.
/// The audit row for a created append lands inside the caller's transaction
/// (a replay audits nothing, mirroring the outbox's exactly-once posture).
///
/// Fails closed on: empty kind, oversized payload, or SQL failure — never
/// truncates, never drops silently.
pub(crate) fn append(
    conn: &Connection,
    run_id: i64,
    kind: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
) -> rusqlite::Result<(bool, i64)> {
    if kind.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "session event kind must be non-empty".into(),
        ));
    }
    if payload_json.len() > PAYLOAD_CAP_BYTES {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "session event payload exceeds cap: {} > {} bytes",
            payload_json.len(),
            PAYLOAD_CAP_BYTES
        )));
    }
    if let Some(existing) = exact(conn, run_id, idempotency_key)? {
        if existing.kind != kind || existing.payload_json != payload_json {
            return Err(refusal("session idempotency key mismatch"));
        }
        return Ok((false, existing.seq));
    }
    // Single statement under the caller's BEGIN IMMEDIATE: seq is computed
    // and the exactly-once guard evaluated atomically, so two racing writers
    // serialize on the write discipline instead of double-drawing a seq.
    // `INSERT OR IGNORE` is the belt to the NOT EXISTS braces — an aggregate
    // SELECT always yields a row, so the guard lives in a scalar subquery.
    let created = conn.execute(
        "INSERT OR IGNORE INTO agent_session_events(run_id, seq, idempotency_key, kind, payload_json, created_at)
         SELECT ?1,
                (SELECT COALESCE(MAX(seq), 0) + 1 FROM agent_session_events WHERE run_id = ?1),
                ?2, ?3, ?4, ?5
          WHERE NOT EXISTS (
              SELECT 1 FROM agent_session_events
               WHERE run_id = ?1 AND idempotency_key = ?2
          )",
        params![run_id, idempotency_key, kind, payload_json, now],
    )? == 1;
    let row =
        exact(conn, run_id, idempotency_key)?.ok_or_else(|| refusal("session append missing"))?;
    if row.kind != kind || row.payload_json != payload_json {
        return Err(refusal("session idempotency key mismatch"));
    }
    let seq = row.seq;
    if created {
        audit_write(
            conn,
            run_id,
            "agent-session",
            AuditStatus::Ok,
            &format!("append:{kind}:{seq}"),
        );
    }
    Ok((created, seq))
}

/// Replay the run's session log: the `cap` MOST RECENT events, returned
/// oldest-first (prefix-stable order). A cap larger than the log returns the
/// whole log; callers wanting the full history for tooling pass an explicit
/// number and own that choice visibly.
pub(crate) fn replay(
    conn: &Connection,
    run_id: i64,
    cap: usize,
) -> rusqlite::Result<Vec<SessionEventRow>> {
    // DESC LIMIT + reverse: the tail window without scanning the prefix.
    let mut stmt = conn.prepare(
        "SELECT seq, kind, payload_json, created_at FROM agent_session_events
          WHERE run_id = ?1 AND kind NOT GLOB 'control:*' ORDER BY seq DESC LIMIT ?2",
    )?;
    let mut rows: Vec<SessionEventRow> = stmt
        .query_map(params![run_id, cap as i64], |r| {
            Ok(SessionEventRow {
                seq: r.get(0)?,
                kind: r.get(1)?,
                payload_json: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.reverse();
    Ok(rows)
}

fn refusal(message: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.into())
}

/// Exact identity lookup is independent of conversational replay. Check byte
/// length before copying payload into Rust (including manually corrupted rows).
/// rusqlite upstream docs verified via Context7 2026-09-17 (unversioned).
pub(crate) fn exact(
    conn: &Connection,
    run_id: i64,
    key: &str,
) -> rusqlite::Result<Option<SessionEventRow>> {
    conn.query_row(
        "SELECT seq, kind, CASE WHEN length(CAST(payload_json AS BLOB)) <= ?3
         THEN payload_json ELSE NULL END, created_at FROM agent_session_events
         WHERE run_id = ?1 AND idempotency_key = ?2",
        params![run_id, key, PAYLOAD_CAP_BYTES as i64],
        |r| {
            Ok(SessionEventRow {
                seq: r.get(0)?,
                kind: r.get(1)?,
                payload_json: r.get(2)?,
                created_at: r.get(3)?,
            })
        },
    )
    .optional()
}

const SCOPE_LOOKUP_CAP: usize = 8192;
const SCOPE_KEY_BYTES: usize = 1024;
const SCOPE_KIND_BYTES: usize = 256;

// No filtering in SQL: even excluded rows spend the bounded lookup budget,
// but only selected narrative spends the caller's conversational window.
const SCOPE_SCAN_SQL: &str = "SELECT seq,
    CASE WHEN length(CAST(kind AS BLOB)) <= ?3 THEN kind ELSE NULL END,
    CASE WHEN length(CAST(payload_json AS BLOB)) <= ?4 THEN payload_json ELSE NULL END,
    created_at,
    CASE WHEN length(CAST(idempotency_key AS BLOB)) <= ?5 THEN idempotency_key ELSE NULL END
    FROM agent_session_events WHERE run_id = ?1 ORDER BY seq DESC LIMIT ?2";

#[derive(Clone, Copy)]
struct ScopeExchange {
    child: bool,
    end_seq: Option<i64>,
}

struct ScopeIdentity<'a> {
    exchange: i64,
    turn: Option<u32>,
    kind: &'static str,
    prefix: &'a str,
}

fn scope_id(value: &str) -> Option<i64> {
    let id = value.parse::<i64>().ok()?;
    (id > 0 && id.to_string() == value).then_some(id)
}

fn scope_turn(value: &str) -> Option<u32> {
    let turn = value.parse::<u32>().ok()?;
    (turn > 0 && turn.to_string() == value).then_some(turn)
}

fn scope_prefix(prefix: &str) -> bool {
    prefix.is_empty()
        || prefix
            .strip_prefix("child:")
            .and_then(|name| name.strip_suffix(':'))
            .is_some_and(|name| !name.is_empty() && name.len() <= 128)
}

// Split from the RIGHT: a display name may itself contain colons and even
// a plausible run/key fragment. The opaque prefix never supplies identity.
fn scope_identity<'a>(key: &'a str, run_marker: &str) -> Option<ScopeIdentity<'a>> {
    let (prefix, suffix) = key.rsplit_once(run_marker)?;
    if !scope_prefix(prefix) {
        return None;
    }
    if let Some(rest) = suffix.strip_prefix("exchange:") {
        if !prefix.is_empty() {
            return None;
        }
        let (id, tail) = rest.split_once(':')?;
        let exchange = scope_id(id)?;
        let (kind, turn) = if tail == "subagent_result" {
            ("subagent_result", None)
        } else {
            (
                "compaction",
                Some(scope_turn(tail.strip_prefix("compact:")?)?),
            )
        };
        return Some(ScopeIdentity {
            exchange,
            turn,
            kind,
            prefix,
        });
    }
    let (head, tail) = suffix.split_once(':')?;
    let (tag, id) = head.split_at_checked(1)?;
    let exchange = scope_id(id)?;
    let (kind, turn) = match tag {
        "u" if tail == "0" => ("user", None),
        "a" => ("assistant", Some(scope_turn(tail.strip_prefix('t')?)?)),
        "c" => ("canceled", Some(scope_turn(tail.strip_prefix('t')?)?)),
        "x" => {
            let (turn, index) = tail.strip_prefix('t')?.split_once(':')?;
            let parsed = index.parse::<u32>().ok()?;
            if parsed >= 64 || parsed.to_string() != index {
                return None;
            }
            ("tool_result", Some(scope_turn(turn)?))
        }
        _ => return None,
    };
    Some(ScopeIdentity {
        exchange,
        turn,
        kind,
        prefix,
    })
}

// Projection contract: `child:` is reserved for delegated request keys, as
// emitted by delegate_owned. Parent callers must not use that namespace.
// Admission itself predates this contract and does not enforce the reservation;
// a conflicting parent transcript refuses rather than being reclassified by its
// display prefix. Explicit admission scope would require a coordinated writer
// and durable-format change, not an inference from the opaque fingerprint.
fn scope_exchange(conn: &Connection, run_id: i64, id: i64) -> rusqlite::Result<ScopeExchange> {
    let (kind, key): (String, String) = conn.query_row(
        "SELECT CASE WHEN length(CAST(kind AS BLOB)) <= 256 THEN kind ELSE NULL END,
         CASE WHEN length(CAST(idempotency_key AS BLOB)) <= 529 THEN idempotency_key ELSE NULL END
         FROM agent_session_events WHERE run_id = ?1 AND seq = ?2",
        params![run_id, id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let request = key
        .strip_prefix("control:exchange:")
        .filter(|request| !request.is_empty() && request.len() <= 512)
        .ok_or_else(|| refusal("context_exchange_start"))?;
    if kind != "control:exchange" || request == "child:" {
        return Err(refusal("context_exchange_start"));
    }
    let end: Option<(i64, String)> = conn
        .query_row(
            "SELECT seq, CASE WHEN length(CAST(kind AS BLOB)) <= 256 THEN kind ELSE NULL END
         FROM agent_session_events WHERE run_id = ?1 AND idempotency_key = ?2",
            params![run_id, format!("control:exchange_done:{id}")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if end
        .as_ref()
        .is_some_and(|(seq, kind)| *seq <= id || kind != "control:exchange_done")
    {
        return Err(refusal("context_exchange_end"));
    }
    Ok(ScopeExchange {
        child: request.starts_with("child:"),
        end_seq: end.map(|(seq, _)| seq),
    })
}

fn scope_exact(
    conn: &Connection,
    run_id: i64,
    key: &str,
) -> rusqlite::Result<Option<SessionEventRow>> {
    if key.len() > SCOPE_KEY_BYTES {
        return Err(refusal("context_key_bounds"));
    }
    conn.query_row(
        "SELECT seq, CASE WHEN length(CAST(kind AS BLOB)) <= ?3 THEN kind ELSE NULL END,
         CASE WHEN length(CAST(payload_json AS BLOB)) <= ?4 THEN payload_json ELSE NULL END,
         created_at FROM agent_session_events WHERE run_id = ?1 AND idempotency_key = ?2",
        params![
            run_id,
            key,
            SCOPE_KIND_BYTES as i64,
            PAYLOAD_CAP_BYTES as i64
        ],
        |r| {
            Ok(SessionEventRow {
                seq: r.get(0)?,
                kind: r.get(1)?,
                payload_json: r.get(2)?,
                created_at: r.get(3)?,
            })
        },
    )
    .optional()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeChildResult {
    name: String,
    exchange_id: i64,
    outcome: String,
    turns: Option<u32>,
    summary: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeEnd {
    version: u32,
    outcome: crate::agentloop::run_loop::RunOutcome,
    assistant_key: Option<String>,
}

fn scope_child_result(
    conn: &Connection,
    run_id: i64,
    row: &SessionEventRow,
    id: i64,
    scope: ScopeExchange,
) -> rusqlite::Result<()> {
    use crate::agentloop::run_loop::RunOutcome;

    let result: ScopeChildResult =
        serde_json::from_str(&row.payload_json).map_err(|_| refusal("context_child_result"))?;
    if !scope.child || result.exchange_id != id || result.name.is_empty() || result.name.len() > 128
    {
        return Err(refusal("context_child_identity"));
    }
    let end = scope_exact(conn, run_id, &format!("control:exchange_done:{id}"))?
        .ok_or_else(|| refusal("context_child_unfinished"))?;
    if scope.end_seq != Some(end.seq) || end.seq >= row.seq {
        return Err(refusal("context_child_order"));
    }
    let receipt: ScopeEnd =
        serde_json::from_str(&end.payload_json).map_err(|_| refusal("context_child_receipt"))?;
    let (outcome, turns) = match receipt.outcome {
        RunOutcome::Completed { turns, .. } => ("completed", Some(turns)),
        RunOutcome::TurnCapReached { turns, .. } => ("capped", Some(turns)),
        RunOutcome::BudgetExceeded { turns, .. } => ("budget_exceeded", Some(turns)),
        RunOutcome::Canceled => ("canceled", None),
    };
    if receipt.version != 1 || result.outcome != outcome || result.turns != turns {
        return Err(refusal("context_child_outcome"));
    }
    // Bind the display name to an actual task key, not to the payload claim.
    let prefix = format!("child:{}:", result.name);
    let user = scope_exact(conn, run_id, &format!("{prefix}run{run_id}:u{id}:0"))?
        .ok_or_else(|| refusal("context_child_task"))?;
    if user.kind != format!("{prefix}user") || user.seq <= id || user.seq >= end.seq {
        return Err(refusal("context_child_task"));
    }
    let text = match receipt.assistant_key {
        Some(key) => {
            if key.len() > SCOPE_KEY_BYTES {
                return Err(refusal("context_child_assistant"));
            }
            let marker = format!("{prefix}run{run_id}:a{id}:t");
            let turn = key
                .strip_prefix(&marker)
                .and_then(scope_turn)
                .ok_or_else(|| refusal("context_child_assistant"))?;
            if turns.is_some_and(|turns| turns != turn) {
                return Err(refusal("context_child_assistant"));
            }
            let assistant = scope_exact(conn, run_id, &key)?
                .ok_or_else(|| refusal("context_child_assistant"))?;
            if assistant.kind != format!("{prefix}assistant")
                || assistant.seq <= user.seq
                || assistant.seq >= end.seq
            {
                return Err(refusal("context_child_assistant"));
            }
            let value: serde_json::Value = serde_json::from_str(&assistant.payload_json)
                .map_err(|_| refusal("context_child_assistant"))?;
            value
                .get("text")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| refusal("context_child_assistant"))?
                .to_string()
        }
        None if turns.is_none_or(|turns| turns == 0) => String::new(),
        None => return Err(refusal("context_child_assistant")),
    };
    if (outcome == "completed" && result.summary.as_deref() != Some(text.as_str()))
        || (outcome != "completed" && result.summary.is_some())
    {
        return Err(refusal("context_child_summary"));
    }
    Ok(())
}

/// Read-only scope selection for trusted, already-authorized run callers.
/// Parent history spans parent exchanges; child history belongs to one admitted
/// exchange, never a name. This is not a tenant/principal authorization API.
/// Legacy parent user/assistant rows survive; unbound child rows never migrate
/// into either scope. Typed envelope/call correlation belongs to context projection.
/// The caller must hold its invocation claim while projecting and using context.
/// Request keys beginning `child:` are reserved for delegated invocations; parent
/// callers using that namespace refuse rather than silently changing scope.
/// Gate evidence (`gdl_gate`) is non-conversational; its validity is checked by
/// the checkpoint reader, not by this projection. Unknown kinds are not gates.
/// A sparse scope beyond the lookup ceiling refuses instead of appearing empty.
#[allow(clippy::too_many_lines)]
pub(crate) fn scoped_replay(
    conn: &Connection,
    run_id: i64,
    cap: usize,
    child_exchange: Option<i64>,
) -> rusqlite::Result<Vec<crate::agentloop::context::ContextEvent>> {
    scoped_replay_inner(conn, run_id, cap, child_exchange, false)
}

/// A capped window must not hide its governing compaction boundary. Search only
/// within the existing candidate ceiling; unavailable context refuses, not guesses.
pub(crate) fn scoped_compaction_replay(
    conn: &Connection,
    run_id: i64,
    cap: usize,
    child_exchange: Option<i64>,
) -> rusqlite::Result<Vec<crate::agentloop::context::ContextEvent>> {
    scoped_replay_inner(conn, run_id, cap, child_exchange, true)
}

#[allow(clippy::too_many_lines)]
fn scoped_replay_inner(
    conn: &Connection,
    run_id: i64,
    cap: usize,
    child_exchange: Option<i64>,
    compaction_aware: bool,
) -> rusqlite::Result<Vec<crate::agentloop::context::ContextEvent>> {
    if run_id <= 0 || !(1..=REPLAY_CAP).contains(&cap) || child_exchange.is_some_and(|id| id <= 0) {
        return Err(refusal("context_scope_bounds"));
    }
    let mut scopes = std::collections::BTreeMap::new();
    if let Some(id) = child_exchange {
        let scope = scope_exchange(conn, run_id, id)?;
        if !scope.child {
            return Err(refusal("context_not_child_exchange"));
        }
        scopes.insert(id, scope);
    }
    let marker = format!("run{run_id}:");
    let mut stmt = conn.prepare(SCOPE_SCAN_SQL)?;
    let mut rows = stmt.query(params![
        run_id,
        (SCOPE_LOOKUP_CAP + 1) as i64,
        SCOPE_KIND_BYTES as i64,
        PAYLOAD_CAP_BYTES as i64,
        SCOPE_KEY_BYTES as i64
    ])?;
    let mut selected = Vec::with_capacity(cap);
    let mut scanned = 0;
    while let Some(r) = rows.next()? {
        if scanned == SCOPE_LOOKUP_CAP {
            return Err(refusal("context_lookup_ceiling"));
        }
        scanned += 1;
        let seq: i64 = r.get(0)?;
        if child_exchange.is_some_and(|id| seq <= id) {
            break;
        }
        let kind: String = r.get(1)?;
        // advance_tx writes gdl_gate as checkpoint evidence under :gate keys.
        // Keep exact checkpoint reads authoritative; never feed artifacts/errors
        // from this evidence envelope into conversation or summary requests.
        if kind.starts_with("control:") || kind == "gdl_gate" {
            continue;
        }
        let key: String = r.get(4)?;
        let identity = scope_identity(&key, &marker);
        let (exchange_id, turn, normalized, scope) = match identity {
            Some(identity) => {
                let scope = if let Some(scope) = scopes.get(&identity.exchange) {
                    *scope
                } else {
                    let scope = scope_exchange(conn, run_id, identity.exchange)?;
                    scopes.insert(identity.exchange, scope);
                    scope
                };
                let prefix = if identity.kind == "compaction" {
                    kind.strip_suffix("compaction")
                        .filter(|prefix| scope_prefix(prefix))
                        .ok_or_else(|| refusal("context_kind_identity"))?
                } else {
                    identity.prefix
                };
                let projected = identity.kind == "subagent_result";
                if kind != format!("{prefix}{}", identity.kind)
                    || (!projected && scope.child == prefix.is_empty())
                    || seq <= identity.exchange
                    || (!projected && scope.end_seq.is_some_and(|end| seq >= end))
                {
                    return Err(refusal("context_kind_identity"));
                }
                let visible = child_exchange.map_or(projected || !scope.child, |id| {
                    !projected && identity.exchange == id
                });
                if !visible || identity.kind == "canceled" {
                    continue;
                }
                (
                    Some(identity.exchange),
                    identity.turn,
                    identity.kind.to_string(),
                    Some(scope),
                )
            }
            None => {
                // A malformed modern key must not downgrade into legacy history,
                // including a child row whose unknown kind or key cannot resolve.
                if key.contains(&marker) || key.starts_with("run") {
                    return Err(refusal("context_event_identity"));
                }
                // An unbound legacy child is exclusion only, never scope. Its
                // undecodable payload cannot be adopted as an empty conversation.
                if kind.starts_with("child:") {
                    continue;
                }
                if child_exchange.is_some() || kind == "canceled" {
                    continue;
                }
                if kind != "user"
                    && kind != "assistant"
                    && !(compaction_aware && kind == "compaction")
                {
                    return Err(refusal("context_unknown_kind"));
                }
                (None, None, kind, None)
            }
        };
        if selected.len() == cap {
            if normalized == "compaction" {
                return Err(refusal("context_compaction_boundary"));
            }
            continue;
        }
        // SQL's byte check runs before any selected payload is copied to Rust.
        let row = SessionEventRow {
            seq,
            kind: normalized,
            payload_json: r.get(2)?,
            created_at: r.get(3)?,
        };
        if row.kind == "subagent_result" {
            let id = exchange_id.ok_or_else(|| refusal("context_child_identity"))?;
            scope_child_result(
                conn,
                run_id,
                &row,
                id,
                scope.ok_or_else(|| refusal("context_child_identity"))?,
            )?;
        }
        selected.push(crate::agentloop::context::ContextEvent {
            row,
            exchange_id,
            turn,
        });
        if selected.len() == cap
            && (!compaction_aware || selected.iter().any(|e| e.row.kind == "compaction"))
        {
            break;
        }
    }
    selected.reverse();
    Ok(selected)
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    version: u32,
    owner: String,
}

fn require_tx(conn: &Connection, owner: &str) -> rusqlite::Result<()> {
    if conn.is_autocommit() || owner.is_empty() || owner.len() > 128 {
        return Err(refusal("claim requires a transaction and bounded owner"));
    }
    Ok(())
}

fn current_claim(conn: &Connection, run_id: i64) -> rusqlite::Result<Option<(i64, Claim)>> {
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT seq, CASE WHEN length(CAST(payload_json AS BLOB)) <= 1024
         THEN payload_json ELSE NULL END FROM agent_session_events
         WHERE run_id = ?1 AND kind = 'control:claim' ORDER BY seq DESC LIMIT 1",
            [run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((seq, payload)) = row else {
        return Ok(None);
    };
    let claim: Claim = serde_json::from_str(&payload).map_err(|_| refusal("corrupt claim"))?;
    if claim.version != 1 || claim.owner.is_empty() || claim.owner.len() > 128 {
        return Err(refusal("unsupported claim"));
    }
    if let Some(released) = exact(conn, run_id, &format!("control:release:{seq}"))? {
        if released.kind != "control:release" || released.payload_json != payload {
            return Err(refusal("corrupt claim release"));
        }
        return Ok(None);
    }
    Ok(Some((seq, claim)))
}

/// Caller MUST hold WorkflowTx (BEGIN IMMEDIATE). No lease, same-owner
/// reentrancy, or crash recovery: an outstanding owner always rejects admission.
pub(crate) fn acquire(conn: &Connection, run_id: i64, owner: &str) -> rusqlite::Result<()> {
    require_tx(conn, owner)?;
    if current_claim(conn, run_id)?.is_some() {
        return Err(refusal("run already owned; explicit recovery required"));
    }
    conn.query_row(
        "SELECT id FROM workflow_runs WHERE id = ?1",
        [run_id],
        |_| Ok(()),
    )?;
    let payload = serde_json::to_string(&Claim {
        version: 1,
        owner: owner.into(),
    })
    .map_err(|_| refusal("claim serialization failed"))?;
    let key = format!("control:claim:{}", uuid::Uuid::new_v4());
    append(
        conn,
        run_id,
        "control:claim",
        &payload,
        &key,
        chrono::Utc::now().timestamp(),
    )?;
    Ok(())
}

pub(crate) fn verify_owner(conn: &Connection, run_id: i64, owner: &str) -> rusqlite::Result<()> {
    require_tx(conn, owner)?;
    match current_claim(conn, run_id)? {
        Some((_, claim)) if claim.owner == owner => Ok(()),
        _ => Err(refusal("run ownership lost or absent")),
    }
}

/// Unsettled synchronous tool work survives waiter cancellation and blocks
/// release/admission until explicit operator recovery (there is no auto-repair).
pub(crate) fn tools_quiescent(conn: &Connection, run_id: i64) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM agent_session_events e
         WHERE e.run_id = ?1 AND e.kind = 'control:tool_intent'
         AND NOT EXISTS (SELECT 1 FROM agent_session_events d WHERE d.run_id=e.run_id
             AND d.idempotency_key=e.idempotency_key || ':done' AND d.kind='control:tool_done'))",
        [run_id],
        |r| r.get(0),
    )
}

/// A run is idle only after its invocation (including receipt/projection),
/// exchange, and every tool are finalized. Check inside the checkpoint tx.
pub(crate) fn quiescent(conn: &Connection, run_id: i64) -> rusqlite::Result<bool> {
    let busy: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM agent_session_events e WHERE e.run_id=?1 AND (
          (e.kind='control:invocation' AND NOT EXISTS (SELECT 1 FROM agent_session_events d
            WHERE d.run_id=e.run_id AND d.idempotency_key=e.idempotency_key || ':done'
            AND d.kind='control:invocation_done')) OR
          (e.kind='control:exchange' AND NOT EXISTS (SELECT 1 FROM agent_session_events d
            WHERE d.run_id=e.run_id AND d.idempotency_key='control:exchange_done:' || e.seq
            AND d.kind='control:exchange_done'))))",
        [run_id],
        |r| r.get(0),
    )?;
    Ok(!busy && tools_quiescent(conn, run_id)?)
}

pub(crate) fn check_idle(conn: &Connection, run_id: i64) -> rusqlite::Result<()> {
    if conn.is_autocommit() || !quiescent(conn, run_id)? {
        return Err(refusal(
            "unfinished invocation, exchange, or tool retains ownership",
        ));
    }
    Ok(())
}

/// Invocation tokens are fresh per call, including terminal retries. They are
/// not request keys: only this token may finalize its receipt/projection.
pub(crate) fn begin_invocation(
    conn: &Connection,
    run_id: i64,
    owner: &str,
    token: &str,
) -> rusqlite::Result<()> {
    verify_owner(conn, run_id, owner)?;
    check_idle(conn, run_id)?;
    if token.is_empty() || token.len() > 128 {
        return Err(refusal("invalid invocation token"));
    }
    let key = format!("control:invocation:{token}");
    if exact(conn, run_id, &key)?.is_some() {
        return Err(refusal("invocation token reused"));
    }
    append(
        conn,
        run_id,
        "control:invocation",
        owner,
        &key,
        chrono::Utc::now().timestamp(),
    )?;
    Ok(())
}

pub(crate) fn verify_invocation(
    conn: &Connection,
    run_id: i64,
    owner: &str,
    token: &str,
) -> rusqlite::Result<()> {
    verify_owner(conn, run_id, owner)?;
    let key = format!("control:invocation:{token}");
    let row = exact(conn, run_id, &key)?.ok_or_else(|| refusal("invocation absent"))?;
    if row.kind != "control:invocation"
        || row.payload_json != owner
        || exact(conn, run_id, &format!("{key}:done"))?.is_some()
    {
        return Err(refusal("invocation owner mismatch or already finalized"));
    }
    Ok(())
}

/// Caller has validated the exact receipt and written any projection in THIS
/// transaction. Also used for a proven pre-exchange admission refusal only.
pub(crate) fn finish_invocation(
    conn: &Connection,
    run_id: i64,
    owner: &str,
    token: &str,
) -> rusqlite::Result<()> {
    verify_invocation(conn, run_id, owner, token)?;
    if !tools_quiescent(conn, run_id)? {
        return Err(refusal("indeterminate tool work retains ownership"));
    }
    append(
        conn,
        run_id,
        "control:invocation_done",
        owner,
        &format!("control:invocation:{token}:done"),
        chrono::Utc::now().timestamp(),
    )?;
    Ok(())
}

/// Atomic exchange admission under the caller's WorkflowTx. A known request
/// must match byte-for-byte; a new request cannot skip an interrupted exchange.
pub(crate) fn admit_exchange(
    conn: &Connection,
    run_id: i64,
    owner: &str,
    key: &str,
    fingerprint: &str,
) -> rusqlite::Result<(bool, i64)> {
    verify_owner(conn, run_id, owner)?;
    if key.is_empty() || key.len() > 512 || !tools_quiescent(conn, run_id)? {
        return Err(refusal("invalid request key or indeterminate tool work"));
    }
    let key = format!("control:exchange:{key}");
    if let Some(row) = exact(conn, run_id, &key)? {
        if row.kind != "control:exchange" || row.payload_json != fingerprint {
            return Err(refusal("exchange request fingerprint mismatch"));
        }
        return Ok((false, row.seq));
    }
    let pending: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM agent_session_events e
         WHERE e.run_id=?1 AND e.kind='control:exchange'
         AND NOT EXISTS (SELECT 1 FROM agent_session_events d WHERE d.run_id=e.run_id
           AND d.idempotency_key='control:exchange_done:' || e.seq AND d.kind='control:exchange_done'))",
        [run_id], |r| r.get(0),
    )?;
    if pending {
        return Err(refusal("interrupted exchange requires explicit recovery"));
    }
    append(
        conn,
        run_id,
        "control:exchange",
        fingerprint,
        &key,
        chrono::Utc::now().timestamp(),
    )
}

/// Only call once all work is known quiescent. In particular a dropped
/// spawn_blocking waiter is NOT evidence that its external tool has stopped.
pub(crate) fn release(conn: &Connection, run_id: i64, owner: &str) -> rusqlite::Result<()> {
    verify_owner(conn, run_id, owner)?;
    check_idle(conn, run_id)?;
    let (seq, claim) = current_claim(conn, run_id)?.ok_or_else(|| refusal("claim absent"))?;
    let payload =
        serde_json::to_string(&claim).map_err(|_| refusal("claim serialization failed"))?;
    append(
        conn,
        run_id,
        "control:release",
        &payload,
        &format!("control:release:{seq}"),
        chrono::Utc::now().timestamp(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::verify_chain;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::ACTOR;
    use crate::workflow::tx::WorkflowTx;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn compaction_aware_replay_refuses_hidden_boundary_without_widening_scope() {
        let mut conn = db();
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        append(tx.tx(), 1, "user", "head", "seed:head", 1).unwrap();
        append(tx.tx(), 1, "user", "tail", "seed:tail", 1).unwrap();
        append(
            tx.tx(),
            1,
            "compaction",
            r#"{"summary":"brief","compacted_through_seq":1,"tail_from_seq":2}"#,
            "seed:compact",
            1,
        )
        .unwrap();
        append(tx.tx(), 1, "user", "new", "seed:new", 1).unwrap();
        assert!(
            scoped_compaction_replay(tx.tx(), 1, 1, None)
                .unwrap_err()
                .to_string()
                .contains("context_compaction_boundary")
        );
        assert_eq!(
            scoped_compaction_replay(tx.tx(), 1, 4, None).unwrap().len(),
            4
        );
        tx.commit().unwrap();
    }

    fn kinds(conn: &Connection, run_id: i64) -> Vec<(i64, String)> {
        replay(conn, run_id, REPLAY_CAP)
            .unwrap()
            .into_iter()
            .map(|r| (r.seq, r.kind))
            .collect()
    }

    fn scope_start(conn: &Connection, request: &str) -> i64 {
        append(
            conn,
            1,
            "control:exchange",
            "fingerprint",
            &format!("control:exchange:{request}"),
            1,
        )
        .unwrap()
        .1
    }

    fn scope_task(conn: &Connection, id: i64, prefix: &str, text: &str) -> i64 {
        append(
            conn,
            1,
            &format!("{prefix}user"),
            text,
            &format!("{prefix}run1:u{id}:0"),
            1,
        )
        .unwrap()
        .1
    }

    fn scope_done(conn: &Connection, id: i64) {
        append(
            conn,
            1,
            "control:exchange_done",
            "{}",
            &format!("control:exchange_done:{id}"),
            1,
        )
        .unwrap();
    }

    #[test]
    fn scoped_replay_excludes_exact_gdl_gate_without_consuming_selected_cap() {
        let conn = db();
        append(&conn, 1, "user", "parent task", "legacy-parent", 1).unwrap();
        let child = scope_start(&conn, "child:gate-fixture");
        scope_task(&conn, child, "child:worker:", "child task");
        for revision in 1..=REPLAY_CAP + 1 {
            // Mirrors advance_tx's kind/key. Payload intentionally isn't a valid
            // checkpoint: only the checkpoint reader may certify that evidence.
            append(
                &conn,
                1,
                "gdl_gate",
                "not conversation",
                &format!("control:gdl:{revision}:gate"),
                1,
            )
            .unwrap();
        }
        let key = "control:gdl:1:gate";
        let before = exact(&conn, 1, key).unwrap().unwrap();
        assert_eq!(
            scoped_replay(&conn, 1, 1, None).unwrap()[0]
                .row
                .payload_json,
            "parent task"
        );
        assert_eq!(
            scoped_replay(&conn, 1, 1, Some(child)).unwrap()[0]
                .row
                .payload_json,
            "child task"
        );
        assert_eq!(exact(&conn, 1, key).unwrap().unwrap(), before);
        assert_eq!(replay(&conn, 1, 1).unwrap()[0].kind, "gdl_gate");
        append(&conn, 1, "gdl_gate_extra", "{}", "unknown-gate", 1).unwrap();
        assert!(scoped_replay(&conn, 1, 1, None).is_err());
    }

    #[test]
    fn scoped_replay_reserves_child_request_namespace_without_changing_admission() {
        let mut conn = db();
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        acquire(tx.tx(), 1, "owner").unwrap();
        let admitted = admit_exchange(tx.tx(), 1, "owner", "child:first", "fingerprint").unwrap();
        assert!(admitted.0);
        scope_task(tx.tx(), admitted.1, "", "parent driver task");
        // Exact admission retries retain their original receipt; only context
        // projection refuses this parent/child namespace contradiction.
        assert_eq!(
            admit_exchange(tx.tx(), 1, "owner", "child:first", "fingerprint").unwrap(),
            (false, admitted.1)
        );
        assert!(scoped_replay(tx.tx(), 1, 10, None).is_err());
        assert!(scoped_replay(tx.tx(), 1, 10, Some(admitted.1)).is_err());
    }

    #[test]
    fn scoped_replay_refuses_malformed_modern_child_but_never_adopts_legacy_child() {
        let conn = db();
        append(&conn, 1, "user", "parent", "legacy-parent", 1).unwrap();
        let child = scope_start(&conn, "child:malformed-fixture");
        scope_task(&conn, child, "child:worker:", "selected task");
        append(
            &conn,
            1,
            "child:worker:assistant",
            "malformed legacy payload",
            "legacy-child",
            1,
        )
        .unwrap();
        assert_eq!(scoped_replay(&conn, 1, 10, Some(child)).unwrap().len(), 1);
        assert_eq!(scoped_replay(&conn, 1, 10, None).unwrap().len(), 1);
        append(
            &conn,
            1,
            "child:worker:assistant",
            "{}",
            &format!("child:worker:run1:a{child}:t0"),
            1,
        )
        .unwrap();
        assert!(scoped_replay(&conn, 1, 10, Some(child)).is_err());
        assert!(scoped_replay(&conn, 1, 10, None).is_err());
    }

    #[test]
    fn scoped_replay_isolates_invocations_not_display_names() {
        let conn = db();
        append(&conn, 1, "user", "legacy parent", "legacy", 1).unwrap();
        let parent = scope_start(&conn, "parent");
        scope_task(&conn, parent, "", "parent task");
        scope_done(&conn, parent);
        let first = scope_start(&conn, "child:first");
        // The name contains a plausible key fragment. Only the final anchored
        // writer suffix is identity, never a guessed split of the display name.
        let prefix = "child:label:run1:a999:t1:";
        scope_task(&conn, first, prefix, "first private task");
        append(
            &conn,
            1,
            &format!("{prefix}assistant"),
            "{}",
            &format!("{prefix}run1:a{first}:t1"),
            1,
        )
        .unwrap();
        append(
            &conn,
            1,
            &format!("{prefix}tool_result"),
            "{}",
            &format!("{prefix}run1:x{first}:t1:0"),
            1,
        )
        .unwrap();
        append(
            &conn,
            1,
            &format!("{prefix}compaction"),
            "{}",
            &format!("run1:exchange:{first}:compact:2"),
            1,
        )
        .unwrap();
        append(
            &conn,
            1,
            &format!("{prefix}canceled"),
            r#"{"turn":2}"#,
            &format!("{prefix}run1:c{first}:t2"),
            1,
        )
        .unwrap();
        scope_done(&conn, first);
        let second = scope_start(&conn, "child:second");
        scope_task(&conn, second, prefix, "second private task");
        // Legacy child rows cannot be adopted by the active child either.
        append(
            &conn,
            1,
            &format!("{prefix}user"),
            "legacy private",
            "old-child",
            1,
        )
        .unwrap();
        let first_rows = scoped_replay(&conn, 1, 10, Some(first)).unwrap();
        assert_eq!(
            first_rows
                .iter()
                .map(|e| e.row.kind.as_str())
                .collect::<Vec<_>>(),
            ["user", "assistant", "tool_result", "compaction"]
        );
        assert!(first_rows.iter().all(|e| e.exchange_id == Some(first)));
        assert_eq!(
            first_rows.iter().map(|e| e.turn).collect::<Vec<_>>(),
            [None, Some(1), Some(1), Some(2)]
        );
        let second_rows = scoped_replay(&conn, 1, 10, Some(second)).unwrap();
        assert_eq!(second_rows.len(), 1);
        assert_eq!(second_rows[0].row.payload_json, "second private task");
        let parent_rows = scoped_replay(&conn, 1, 10, None).unwrap();
        assert_eq!(
            parent_rows
                .iter()
                .map(|e| e.row.payload_json.as_str())
                .collect::<Vec<_>>(),
            ["legacy parent", "parent task"]
        );
        assert_eq!(parent_rows[0].exchange_id, None);
        assert_eq!(parent_rows[1].exchange_id, Some(parent));
        assert!(scoped_replay(&conn, 1, 10, Some(parent)).is_err());
    }

    #[test]
    fn scoped_replay_selects_before_cap_and_refuses_lookup_exhaustion() {
        let conn = db();
        append(&conn, 1, "user", "parent", "legacy", 1).unwrap();
        let first = scope_start(&conn, "child:first");
        scope_task(&conn, first, "child:same:", "selected");
        scope_done(&conn, first);
        let sibling = scope_start(&conn, "child:sibling");
        scope_task(&conn, sibling, "child:same:", "sibling");
        for turn in 1..=501 {
            append(
                &conn,
                1,
                "child:same:assistant",
                "{}",
                &format!("child:same:run1:a{sibling}:t{turn}"),
                1,
            )
            .unwrap();
        }
        for index in 0..501 {
            append(
                &conn,
                1,
                "control:test",
                "{}",
                &format!("control:flood:{index}"),
                1,
            )
            .unwrap();
        }
        assert_eq!(
            scoped_replay(&conn, 1, 1, None).unwrap()[0]
                .row
                .payload_json,
            "parent"
        );
        assert_eq!(
            scoped_replay(&conn, 1, 1, Some(first)).unwrap()[0]
                .row
                .payload_json,
            "selected"
        );
        // A cheap synthetic flood exercises the work ceiling without audit work.
        conn.execute(
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x < ?1)
             INSERT INTO agent_session_events(run_id, seq, idempotency_key, kind, payload_json, created_at)
             SELECT 1, 10000+x, 'flood:' || x, 'control:test', '{}', 1 FROM n",
            [SCOPE_LOOKUP_CAP as i64],
        ).unwrap();
        let err = scoped_replay(&conn, 1, 1, None).unwrap_err();
        assert!(err.to_string().contains("context_lookup_ceiling"));
        assert!(scoped_replay(&conn, 1, 1, Some(first)).is_err());
        append(&conn, 1, "user", "recent", "recent-legacy", 1).unwrap();
        assert_eq!(
            scoped_replay(&conn, 1, 1, None).unwrap()[0]
                .row
                .payload_json,
            "recent"
        );
    }

    #[test]
    fn scoped_replay_exact_lookup_budget_can_reach_end() {
        let conn = db();
        conn.execute(
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x < ?1)
             INSERT INTO agent_session_events(run_id, seq, idempotency_key, kind, payload_json, created_at)
             SELECT 1, x, 'control:' || x, 'control:test', '{}', 1 FROM n",
            [SCOPE_LOOKUP_CAP as i64],
        ).unwrap();
        assert!(scoped_replay(&conn, 1, 1, None).unwrap().is_empty());
    }

    fn scope_result_fixture(conn: &Connection) -> (i64, String) {
        let id = scope_start(conn, "child:result");
        scope_task(conn, id, "child:worker:", "private task");
        let assistant = format!("child:worker:run1:a{id}:t1");
        append(
            conn,
            1,
            "child:worker:assistant",
            r#"{"text":"public summary","tool_calls":[]}"#,
            &assistant,
            1,
        )
        .unwrap();
        let outcome = crate::agentloop::run_loop::RunOutcome::Completed {
            turns: 1,
            usage: crate::agentloop::provider::Usage::default(),
        };
        let end = serde_json::json!({"version":1, "outcome":outcome, "assistant_key":assistant});
        append(
            conn,
            1,
            "control:exchange_done",
            &end.to_string(),
            &format!("control:exchange_done:{id}"),
            1,
        )
        .unwrap();
        let payload = serde_json::json!({"name":"worker", "exchange_id":id,
            "outcome":"completed", "turns":1, "summary":"public summary"})
        .to_string();
        append(
            conn,
            1,
            "subagent_result",
            &payload,
            &format!("run1:exchange:{id}:subagent_result"),
            1,
        )
        .unwrap();
        (id, payload)
    }

    #[test]
    fn scoped_replay_child_projection_verifies_durable_relationships_and_is_read_only() {
        let conn = db();
        let (id, payload) = scope_result_fixture(&conn);
        let before = replay(&conn, 1, 500).unwrap();
        let changes = conn.changes();
        let projected = scoped_replay(&conn, 1, 500, None).unwrap();
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].exchange_id, Some(id));
        assert_eq!(projected[0].row.kind, "subagent_result");
        assert_eq!(projected[0].row.payload_json, payload);
        assert_eq!(conn.changes(), changes);
        assert_eq!(replay(&conn, 1, 500).unwrap(), before);
        let key = format!("run1:exchange:{id}:subagent_result");
        let original = exact(&conn, 1, &key).unwrap().unwrap();
        assert_eq!(
            append(&conn, 1, "subagent_result", &payload, &key, 2).unwrap(),
            (false, original.seq)
        );
        for (field, value) in [
            ("exchange_id", serde_json::json!(id + 1)),
            ("name", serde_json::json!("other")),
            ("summary", serde_json::json!("different")),
            ("outcome", serde_json::json!("canceled")),
            ("turns", serde_json::json!(2)),
        ] {
            let mut changed: serde_json::Value = serde_json::from_str(&payload).unwrap();
            changed[field] = value;
            conn.execute("UPDATE agent_session_events SET payload_json=?1 WHERE run_id=1 AND idempotency_key=?2",
                params![changed.to_string(), key]).unwrap();
            assert!(scoped_replay(&conn, 1, 500, None).is_err());
        }
        conn.execute(
            "UPDATE agent_session_events SET payload_json=?1 WHERE run_id=1 AND idempotency_key=?2",
            params![payload, key],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM agent_session_events WHERE run_id=1 AND idempotency_key=?1",
            [format!("control:exchange_done:{id}")],
        )
        .unwrap();
        assert!(scoped_replay(&conn, 1, 500, None).is_err());
    }

    #[test]
    fn scoped_replay_refuses_bad_bounds_visible_kinds_and_modern_keys() {
        let conn = db();
        for (run, cap, child) in [
            (0, 1, None),
            (1, 0, None),
            (1, 501, None),
            (1, 1, Some(0)),
            (1, 1, Some(99)),
        ] {
            assert!(scoped_replay(&conn, run, cap, child).is_err());
        }
        append(&conn, 1, "mystery", "{}", "legacy", 1).unwrap();
        assert!(scoped_replay(&conn, 1, 1, None).is_err());
        conn.execute("DELETE FROM agent_session_events", [])
            .unwrap();
        for key in [
            "run2:u1:0",
            "run1:u999:0",
            "run1:a1:t0",
            "run1:a01:t1",
            "run1:x1:t1:64",
        ] {
            append(&conn, 1, "user", "not legacy", key, 1).unwrap();
            assert!(scoped_replay(&conn, 1, 1, None).is_err());
            conn.execute("DELETE FROM agent_session_events", [])
                .unwrap();
        }
        let id = scope_start(&conn, "child:other-run");
        scope_task(&conn, id, "child:worker:", "private");
        assert!(scoped_replay(&conn, 2, 1, Some(id)).is_err());
        assert!(scoped_replay(&conn, 2, 1, None).unwrap().is_empty());
    }

    #[test]
    fn scoped_replay_checks_payload_bytes_before_copying_selected_content() {
        let conn = db();
        append(&conn, 1, "user", "fine", "legacy", 1).unwrap();
        conn.execute(
            "UPDATE agent_session_events SET payload_json=?1",
            ["é".repeat(PAYLOAD_CAP_BYTES)],
        )
        .unwrap();
        assert!(scoped_replay(&conn, 1, 1, None).is_err());
        let id = scope_start(&conn, "child:bounded");
        scope_task(&conn, id, "child:worker:", "visible child");
        // Oversized out-of-scope parent text is never copied or decoded.
        assert_eq!(scoped_replay(&conn, 1, 1, Some(id)).unwrap().len(), 1);
        conn.execute(
            "UPDATE agent_session_events SET payload_json=?1 WHERE idempotency_key=?2",
            params![
                "é".repeat(PAYLOAD_CAP_BYTES),
                format!("child:worker:run1:u{id}:0")
            ],
        )
        .unwrap();
        assert!(scoped_replay(&conn, 1, 1, Some(id)).is_err());
    }

    #[test]
    fn scoped_replay_queries_use_existing_run_indexes_without_sorting() {
        let conn = db();
        let mut stmt = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {SCOPE_SCAN_SQL}"))
            .unwrap();
        let plan = stmt
            .query_map(
                params![
                    1,
                    (SCOPE_LOOKUP_CAP + 1) as i64,
                    SCOPE_KIND_BYTES as i64,
                    PAYLOAD_CAP_BYTES as i64,
                    SCOPE_KEY_BYTES as i64
                ],
                |r| r.get::<_, String>(3),
            )
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("\n");
        assert!(plan.contains("SEARCH agent_session_events USING"), "{plan}");
        assert!(plan.contains("run_id=?"), "{plan}");
        assert!(
            !plan.contains("SCAN ") && !plan.contains("TEMP B-TREE"),
            "{plan}"
        );
        for predicate in ["seq = ?2", "idempotency_key = ?2"] {
            let mut stmt = conn.prepare(&format!(
                "EXPLAIN QUERY PLAN SELECT kind, payload_json FROM agent_session_events WHERE run_id = ?1 AND {predicate}"
            )).unwrap();
            let plan = stmt
                .query_map(params![1, 1], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .join("\n");
            assert!(plan.contains("SEARCH agent_session_events USING"), "{plan}");
            assert!(
                !plan.contains("SCAN ") && !plan.contains("TEMP B-TREE"),
                "{plan}"
            );
        }
    }

    #[test]
    fn r1_cross_process_claim_probe() {
        let Ok(path) = std::env::var("BRAIN_R1_CLAIM_TEST_DB") else {
            return;
        };
        let mut conn = Connection::open(path).unwrap();
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        assert!(acquire(tx.tx(), 1, "child-process").is_err());
        verify_owner(tx.tx(), 1, "parent-process").unwrap();
    }

    #[test]
    fn r1_claim_excludes_an_independent_process() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut destination = Connection::open(file.path()).unwrap();
        run_migration(&mut destination, 1).unwrap();
        destination
            .execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
                [],
            )
            .unwrap();
        let mut tx = WorkflowTx::begin(&mut destination).unwrap();
        acquire(tx.tx(), 1, "parent-process").unwrap();
        tx.commit().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "workflow::session_log::tests::r1_cross_process_claim_probe",
                "--test-threads=1",
                "--nocapture",
            ])
            .env("BRAIN_R1_CLAIM_TEST_DB", file.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("1 passed"));
    }

    #[test]
    fn r1_terminal_exchange_still_blocks_idle_until_invocation_finalized() {
        let mut conn = db();
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        acquire(tx.tx(), 1, "owner").unwrap();
        begin_invocation(tx.tx(), 1, "owner", "first").unwrap();
        let (_, id) = admit_exchange(tx.tx(), 1, "owner", "request", "fingerprint").unwrap();
        append(
            tx.tx(),
            1,
            "control:exchange_done",
            "{}",
            &format!("control:exchange_done:{id}"),
            1,
        )
        .unwrap();
        assert!(tools_quiescent(tx.tx(), 1).unwrap());
        assert!(!quiescent(tx.tx(), 1).unwrap());
        assert!(begin_invocation(tx.tx(), 1, "owner", "second").is_err());
        assert!(release(tx.tx(), 1, "owner").is_err());
        assert!(check_idle(tx.tx(), 1).is_err());
        finish_invocation(tx.tx(), 1, "owner", "first").unwrap();
        assert!(finish_invocation(tx.tx(), 1, "owner", "first").is_err());
        check_idle(tx.tx(), 1).unwrap();
        release(tx.tx(), 1, "owner").unwrap();
    }

    #[test]
    fn r1_claims_require_transactions_refuse_reentry_and_rollback() {
        let mut conn = db();
        assert!(acquire(&conn, 1, "a").is_err());
        {
            let mut tx = WorkflowTx::begin(&mut conn).unwrap();
            acquire(tx.tx(), 1, "a").unwrap();
        }
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        acquire(tx.tx(), 1, "a").unwrap();
        assert!(acquire(tx.tx(), 1, "a").is_err());
        assert!(acquire(tx.tx(), 1, "b").is_err());
        assert!(release(tx.tx(), 1, "b").is_err());
        verify_owner(tx.tx(), 1, "a").unwrap();
        release(tx.tx(), 1, "a").unwrap();
        acquire(tx.tx(), 1, "b").unwrap();
        tx.commit().unwrap();
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        verify_owner(tx.tx(), 1, "b").unwrap();
        assert!(acquire(tx.tx(), 1, "b").is_err());
    }

    #[test]
    fn r1_exact_read_bounds_and_claim_corruption_refuse() {
        let mut conn = db();
        append(&conn, 1, "user", "fine", "k", 1).unwrap();
        conn.execute(
            "UPDATE agent_session_events SET payload_json=?1 WHERE idempotency_key='k'",
            ["é".repeat(PAYLOAD_CAP_BYTES)],
        )
        .unwrap();
        assert!(exact(&conn, 1, "k").is_err());
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        append(tx.tx(), 1, "control:claim", "{}", "bad", 1).unwrap();
        assert!(acquire(tx.tx(), 1, "a").is_err());
    }

    #[test]
    fn r1_control_flood_never_enters_compaction_or_spends_replay_window() {
        let conn = db();
        append(&conn, 1, "user", "keep this conversation", "user", 1).unwrap();
        for index in 0..=REPLAY_CAP {
            append(
                &conn,
                1,
                "control:test",
                "not conversation",
                &format!("ctrl:{index}"),
                2,
            )
            .unwrap();
        }
        let events = replay(&conn, 1, 1).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload_json, "keep this conversation");
        assert!(crate::agentloop::compaction::plan(&events).is_none());
        assert!(exact(&conn, 1, "user").unwrap().is_some());
    }

    #[test]
    fn r1_interrupted_exchange_refuses_same_and_new_keys_without_new_rows() {
        let mut conn = db();
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        acquire(tx.tx(), 1, "owner").unwrap();
        let first = admit_exchange(tx.tx(), 1, "owner", "request", "fingerprint").unwrap();
        assert!(first.0);
        assert_eq!(
            admit_exchange(tx.tx(), 1, "owner", "request", "fingerprint").unwrap(),
            (false, first.1)
        );
        assert!(
            exact(tx.tx(), 1, &format!("control:exchange_done:{}", first.1))
                .unwrap()
                .is_none()
        );
        assert!(admit_exchange(tx.tx(), 1, "owner", "different", "fingerprint").is_err());
        assert!(admit_exchange(tx.tx(), 1, "owner", "request", "changed").is_err());
        let count: i64 = tx
            .tx()
            .query_row("SELECT count(*) FROM agent_session_events", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn r1_changed_key_refuses_and_controls_do_not_consume_replay() {
        let conn = db();
        append(&conn, 1, "user", "original", "original", 1).unwrap();
        assert!(append(&conn, 1, "user", "different", "original", 2).is_err());
        append(&conn, 1, "control:test", "{}", "control", 3).unwrap();
        assert_eq!(replay(&conn, 1, 1).unwrap()[0].kind, "user");
    }

    #[test]
    fn appends_are_sequential_and_audited() {
        let conn = db();
        for (i, kind) in ["user", "assistant", "tool_result"].iter().enumerate() {
            let (created, seq) =
                append(&conn, 1, kind, "{}", &format!("k-{i}"), 100 + i as i64).unwrap();
            assert!(created);
            assert_eq!(seq, i as i64 + 1, "per-run seq is monotonic from 1");
        }
        assert_eq!(
            kinds(&conn, 1),
            vec![
                (1, "user".into()),
                (2, "assistant".into()),
                (3, "tool_result".into()),
            ]
        );
        let audit: Vec<(String, bool)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT actor, detail_hash IS NOT NULL FROM audit_events
                      WHERE kind = 'workflow' ORDER BY id",
                )
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(audit.len(), 3, "one audit row per created append");
        assert!(
            audit.iter().all(|(actor, _)| actor == ACTOR),
            "substrate audit identity stamps session writes"
        );
        assert!(verify_chain(&conn), "the audit chain still verifies");
    }

    #[test]
    fn append_is_idempotent_by_key_and_audits_once() {
        let conn = db();
        let first = append(&conn, 1, "user", r#"{"q":"hi"}"#, "k-dup", 1).unwrap();
        let replayed = append(&conn, 1, "user", r#"{"q":"hi"}"#, "k-dup", 2).unwrap();
        assert!(first.0);
        assert!(!replayed.0, "a known key is a no-op receipt");
        assert_eq!(first.1, replayed.1, "the receipt returns the ORIGINAL seq");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM agent_session_events", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1);
        let audits: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audits, 1, "the replay audits nothing (exactly-once)");
    }

    #[test]
    fn append_rolls_back_with_the_transition() {
        let mut conn = db();
        {
            let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
            append(wtx.tx(), 1, "user", "{}", "k-tx", 1).unwrap();
            // Drop without commit: the event AND its audit row must vanish.
        }
        assert!(
            kinds(&conn, 1).is_empty(),
            "a rolled-back append leaves no session row"
        );
        let audits: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audits, 0, "and no audit row claiming it happened");
    }

    #[test]
    fn replay_returns_the_recent_window_oldest_first() {
        let conn = db();
        for i in 0..6 {
            append(&conn, 1, "user", "{}", &format!("k-{i}"), i).unwrap();
        }
        let seqs: Vec<i64> = replay(&conn, 1, 4)
            .unwrap()
            .into_iter()
            .map(|r| r.seq)
            .collect();
        assert_eq!(seqs, vec![3, 4, 5, 6], "the 4 most recent, oldest-first");
    }

    #[test]
    fn runs_do_not_share_a_log() {
        let conn = db();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        append(&conn, 1, "user", "{}", "k-one", 1).unwrap();
        append(&conn, 2, "user", "{}", "k-two", 1).unwrap();
        assert_eq!(kinds(&conn, 1).len(), 1);
        assert_eq!(kinds(&conn, 2).len(), 1, "per-run logs are isolated");
    }

    #[test]
    fn bounds_are_pinned_and_refuse_loud() {
        assert_eq!(REPLAY_CAP, 500);
        assert_eq!(PAYLOAD_CAP_BYTES, 64 * 1024);
        let conn = db();
        let oversized = "x".repeat(PAYLOAD_CAP_BYTES + 1);
        assert!(
            append(&conn, 1, "tool_result", &oversized, "k-big", 1).is_err(),
            "an oversized payload is refused, never truncated"
        );
        assert!(
            append(&conn, 1, "", "{}", "k-empty", 1).is_err(),
            "an empty kind is refused"
        );
        assert!(
            kinds(&conn, 1).is_empty(),
            "refused appends leave no rows behind"
        );
    }

    #[test]
    fn audit_rows_carry_the_runs_tenant() {
        let conn = db();
        append(&conn, 1, "user", "{}", "k-tenant", 1).unwrap();
        let tenant: String = conn
            .query_row(
                "SELECT tenant_id FROM audit_events WHERE kind = 'workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            tenant, "acme",
            "session writes audit under the run's domain"
        );
    }
}
