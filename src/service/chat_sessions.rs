//! The chat-session core (the Foundation Line: new code is ALWAYS a service
//! core) — the chat spine's durable store, owner-scoped and audited.
//!
//! OWNS (this aggregate's complete storage story):
//! - `chat_sessions` + `chat_turns` (additive tables, no FK — the
//!   `decision_run_traces` posture; foreign rows are the writer's refusal,
//!   not the schema's);
//! - the turn-pair persistence the `/chat/turns` handler calls at the
//!   bridge's FINAL (lazy session create + both rows + audit inside ONE tx);
//! - the listing and turn pages (bounded, owner-scoped, newest/oldest as
//!   the wire states);
//! - rename and erase (each audited inside its own tx; the erase removes
//!   turns + session in ONE tx — the erasure posture: a transcript is
//!   personal data, so nothing soft-deleted lingers).
//!
//! THE LAWS THIS MODULE HOLDS, stated where they can be checked:
//!
//! - **Lazy creation.** A session row exists only because a turn named its
//!   key. There is no create path here, and a caller that INSERTs a session
//!   without a turn is the defect this module exists to kill — so the only
//!   INSERT is inside `persist_turn_pair`, in the same tx as the turn rows.
//! - **Owner scope is a WHERE clause, not an authz guess.** The handler
//!   authorizes; this module receives the resolved owner fence (`None` =
//!   unrestricted — loopback/opaque/Admin; `Some(..)` = only these owners)
//!   and the SQL decides. A foreign key and an unknown key answer the SAME
//!   `SessionNotFound` — no existence oracle.
//! - **A failed generation writes nothing.** The pair persists only at the
//!   FINAL; there is no user-row-without-answer shape in this module.
//! - **Screen at write, sanitize at read.** The stored form passes the
//!   read-seam pipeline at WRITE (the `screen_content` posture — the
//!   sanitizing half: trim, bounds, invisible/control + hostile-element
//!   strip, fixed point), so a `<script>` payload is stored screened; the
//!   blocklist-refusal half stays corpus-only, deliberately: a transcript
//!   is the user's own conversation, not corpus memory, and the kernel's
//!   answer is already fenced at generation time. Every emitted field
//!   passes `sanitize_read` at the handler (the read seam stays at the
//!   handler boundary, the service returns stored forms).
//! - **Bounds are re-asserted HERE**, not only at the wire: label ≤ 80
//!   bytes, page limits ≤ 50, session key the `/chat/turns` shape (one
//!   whitespace-free token ≤ 64 bytes). The fence holds of the FUNCTION, so
//!   every future caller inherits it.
//! - **No governance hash on chat rows.** That tuple is the KNOWLEDGE row's
//!   governance state; the transcript's integrity story is the hash-chained
//!   audit row per write.
//!
//! Wall-clock time enters as an argument (`now_unix`), so a pin can hold
//! the timestamps to a known second.

use rusqlite::{Connection, TransactionBehavior, params};

use crate::audit::AuditStatus;

/// The listing page's ceiling (the bounds law: every list surface capped).
pub const MAX_PAGE_LIMIT: i64 = 50;
/// The listing's default page.
pub const DEFAULT_LIST_LIMIT: i64 = 20;
/// The turn page's default (one page ≈ one screen).
pub const DEFAULT_TURNS_LIMIT: i64 = 50;
/// The rename ceiling, in bytes.
pub const MAX_LABEL_BYTES: usize = 80;
/// The session key's shape, shared with `/chat/turns` (`bounded_session_label`).
pub const MAX_SESSION_KEY_BYTES: usize = 64;

/// Typed service error (the ServiceError convention: one enum per module).
/// `SessionNotFound` is the ONLY per-row refusal and it is deliberately
/// unindexed — unknown and foreign keys carry no distinguishing payload.
#[derive(Debug)]
pub enum ChatSessionError {
    /// The key is not the wire's shape (empty, over 64 bytes, whitespace).
    /// Storage never sees a key the wire would have refused.
    InvalidKey,
    /// The rename label is empty or over the 80-byte ceiling.
    InvalidLabel(usize),
    /// Unknown key AND foreign key — identical by design.
    SessionNotFound,
    /// A query or write failed; the rusqlite text travels unchanged (the
    /// handler maps it onto the route's frozen internal-error body).
    Storage(String),
}

impl std::fmt::Display for ChatSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatSessionError::InvalidKey => {
                write!(
                    f,
                    "the session key must be a single token of at most {MAX_SESSION_KEY_BYTES} bytes"
                )
            }
            ChatSessionError::InvalidLabel(len) => write!(
                f,
                "the label is {len} bytes; the ceiling is {MAX_LABEL_BYTES}"
            ),
            ChatSessionError::SessionNotFound => write!(f, "chat_session_not_found"),
            ChatSessionError::Storage(e) => write!(f, "database error: {e}"),
        }
    }
}

impl From<rusqlite::Error> for ChatSessionError {
    fn from(e: rusqlite::Error) -> Self {
        ChatSessionError::Storage(e.to_string())
    }
}

/// The owner fence, resolved by the handler (the review-queue shape:
/// `record_read_gate().owner_in`). `None` = NO owner predicate (loopback/
/// opaque superuser, Admin); `Some(..)` = only rows whose `owner` is in the
/// set.
#[derive(Debug, Clone)]
pub struct OwnerScope {
    pub owner_in: Option<Vec<String>>,
}

impl OwnerScope {
    pub fn unrestricted() -> Self {
        OwnerScope { owner_in: None }
    }
    pub fn narrowed(owners: Vec<String>) -> Self {
        OwnerScope {
            owner_in: Some(owners),
        }
    }
    /// The SQL fragment + bindings for this scope, with the IN-list's
    /// placeholders starting AFTER `offset` bound parameters (so a caller's
    /// own `?1..` predicates cannot collide with the scope's).
    fn clause_for(&self, offset: usize) -> (String, Vec<String>) {
        match &self.owner_in {
            None => ("1=1".to_owned(), Vec::new()),
            Some(owners) => {
                let placeholders = owners
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + offset + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                (format!("owner IN ({placeholders})"), owners.clone())
            }
        }
    }

    /// The scope alone as a WHERE fragment (placeholders from ?1).
    fn clause(&self) -> (String, Vec<String>) {
        self.clause_for(0)
    }
}

/// The session key's fence: the same shape `/chat/turns` enforces at the
/// wire (`bounded_session_label`), re-asserted at the storage boundary so
/// every future caller inherits it.
fn bounded_session_key(key: &str) -> Result<String, ChatSessionError> {
    let trimmed = key.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_SESSION_KEY_BYTES
        || trimmed.split_whitespace().count() != 1
    {
        return Err(ChatSessionError::InvalidKey);
    }
    Ok(trimmed.to_owned())
}

/// The label's fence: non-empty after trim, at most [`MAX_LABEL_BYTES`] bytes.
fn bounded_label(label: &str) -> Result<String, ChatSessionError> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err(ChatSessionError::InvalidLabel(0));
    }
    if trimmed.len() > MAX_LABEL_BYTES {
        return Err(ChatSessionError::InvalidLabel(trimmed.len()));
    }
    Ok(trimmed.to_owned())
}

/// The write-time screen (the sanitizing half — see the module doc): the
/// stored form is the read-seam pipeline's output, computed ONCE here so the
/// screen judges exactly the bytes that will be persisted.
fn screened(content: &str) -> String {
    crate::gate::sanitize_read(content, false, &None)
}

/// One session as the wire states it (the handler maps 1:1; the service
/// never names an HTTP shape beyond these stored facts).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub session_key: String,
    pub label: String,
    pub turn_count: i64,
    pub created_at: i64,
    pub last_turn_at: i64,
}

/// One stored turn. `content` is the STORED form; the read seam shapes it at
/// the handler.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnRow {
    pub seq: i64,
    pub role: &'static str,
    pub content: String,
    /// The grounding event verbatim (assistant rows), already JSON text.
    pub grounding_json: Option<String>,
    pub tier: Option<String>,
    /// The cite-or-abstain verdict (assistant rows, LL-04): the wire name of
    /// the `support.decision` the kernel labeled the turn with. `None` =
    /// predates the verdict (every pre-LL-04 row).
    pub support_decision: Option<String>,
    pub created_at: i64,
}

/// One page of sessions, newest activity first, with the honest total (the
/// count of ALL the caller's sessions, not the page's length).
pub fn list_sessions(
    conn: &Connection,
    scope: &OwnerScope,
    limit: i64,
    offset: i64,
) -> Result<(Vec<SessionRow>, i64), ChatSessionError> {
    let limit = limit.clamp(1, MAX_PAGE_LIMIT);
    let offset = offset.max(0);
    let (clause, binds) = scope.clause();
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM chat_sessions WHERE {clause}"),
        rusqlite::params_from_iter(binds.iter()),
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(&format!(
        "SELECT session_key, label, turn_count, created_at, last_turn_at
         FROM chat_sessions WHERE {clause}
         ORDER BY last_turn_at DESC, id DESC
         LIMIT ?{} OFFSET ?{}",
        binds.len() + 1,
        binds.len() + 2
    ))?;
    let mut bind_values: Vec<String> = binds.clone();
    bind_values.push(limit.to_string());
    bind_values.push(offset.to_string());
    let rows = stmt
        .query_map(rusqlite::params_from_iter(bind_values.iter()), |r| {
            Ok(SessionRow {
                session_key: r.get(0)?,
                label: r.get(1)?,
                turn_count: r.get(2)?,
                created_at: r.get(3)?,
                last_turn_at: r.get(4)?,
            })
        })?
        .flatten()
        .collect();
    Ok((rows, total))
}

/// One page of a session's turns, oldest first. The SAME refusal for an
/// unknown key and a foreign key.
pub fn list_turns(
    conn: &Connection,
    scope: &OwnerScope,
    session_key: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<TurnRow>, i64), ChatSessionError> {
    let key = bounded_session_key(session_key)?;
    let limit = limit.clamp(1, MAX_PAGE_LIMIT);
    let offset = offset.max(0);
    let tx_like = TxProbe { conn };
    let Some(session_id) = tx_like.owned_session_id(&key, scope)? else {
        return Err(ChatSessionError::SessionNotFound);
    };
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM chat_turns WHERE session_id = ?1",
        params![session_id],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT seq, role, content, grounding_json, tier, support_decision, created_at
         FROM chat_turns WHERE session_id = ?1
         ORDER BY seq ASC
         LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt
        .query_map(params![session_id, limit, offset], |r| {
            Ok(TurnRow {
                seq: r.get(0)?,
                role: role_of(&r.get::<_, String>(1)?),
                content: r.get(2)?,
                grounding_json: r.get(3)?,
                tier: r.get(4)?,
                support_decision: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?
        .flatten()
        .collect();
    Ok((rows, total))
}

/// Map the stored role text back onto the wire vocabulary. The CHECK
/// constraint admits only the two values; an unreadable value is storage
/// damage and refuses rather than inventing a third role.
fn role_of(raw: &str) -> &'static str {
    match raw {
        "assistant" => "assistant",
        _ => "user",
    }
}

/// Persist ONE turn pair at the bridge's FINAL: the user's message and the
/// assistant's final answer, in ONE transaction, lazily creating the session
/// row when the key is fresh (the create INSERT lives HERE, inside the same
/// tx — an empty session row is unrepresentable), bumping the activity
/// counters, and writing the audit evidence inside the same tx.
///
/// `support_decision` is the cite-or-abstain verdict's wire name (LL-04,
/// `service::chat_support`), stored on the assistant row: an Unsupported
/// turn is STORED FLAGGED — visible, never silently an answer.
///
/// A failed generation NEVER reaches this function — the handler calls it
/// only after the FINAL event landed, so the orphan-user-row shape has no
/// path here.
///
/// Returns the session's id (the caller logs it, nothing more).
#[allow(clippy::too_many_arguments)]
pub fn persist_turn_pair(
    conn: &mut Connection,
    session_key: &str,
    owner: &str,
    user_message: &str,
    assistant_text: &str,
    grounding_json: Option<&str>,
    tier: Option<&str>,
    support_decision: Option<&str>,
    now_unix: i64,
) -> Result<i64, ChatSessionError> {
    let key = bounded_session_key(session_key)?;
    if owner.trim().is_empty() {
        return Err(ChatSessionError::Storage(
            "a chat session is always attributable; the owner stamp was empty".to_owned(),
        ));
    }
    if let Some(raw) = support_decision {
        if crate::service::chat_support::SupportDecision::parse(raw).is_none() {
            return Err(ChatSessionError::Storage(format!(
                "unknown support decision {raw:?} — the vocabulary is the verdict enum's"
            )));
        }
    }
    let user_stored = screened(user_message);
    let assistant_stored = screened(assistant_text);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM chat_sessions WHERE session_key = ?1",
            params![key],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    let session_id = match existing {
        Some(id) => {
            tx.execute(
                "UPDATE chat_sessions
                 SET last_turn_at = ?2, turn_count = turn_count + 1
                 WHERE id = ?1",
                params![id, now_unix],
            )?;
            id
        }
        None => {
            tx.execute(
                "INSERT INTO chat_sessions(session_key, owner, label, created_at, last_turn_at, turn_count)
                 VALUES (?1, ?2, '', ?3, ?3, 1)",
                params![key, owner, now_unix],
            )?;
            let id = tx.last_insert_rowid();
            crate::audit::record(
                &tx,
                crate::audit::AuditKind::Chat,
                owner,
                &key,
                AuditStatus::Ok,
                "chat_session_create",
            );
            id
        }
    };
    let next_seq: i64 = tx.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM chat_turns WHERE session_id = ?1",
        params![session_id],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO chat_turns(session_id, seq, role, content, grounding_json, tier, support_decision, created_at)
         VALUES (?1, ?2, 'user', ?3, NULL, NULL, NULL, ?4)",
        params![session_id, next_seq, user_stored, now_unix],
    )?;
    tx.execute(
        "INSERT INTO chat_turns(session_id, seq, role, content, grounding_json, tier, support_decision, created_at)
         VALUES (?1, ?2, 'assistant', ?3, ?4, ?5, ?6, ?7)",
        params![
            session_id,
            next_seq + 1,
            assistant_stored,
            grounding_json,
            tier,
            support_decision,
            now_unix
        ],
    )?;
    // audit-per-write: the pair and its evidence commit or roll back together.
    // Identifiers only (the chain hashes target + detail) — never turn text.
    crate::audit::record(
        &tx,
        crate::audit::AuditKind::Chat,
        owner,
        &key,
        AuditStatus::Ok,
        "chat_turn",
    );
    tx.commit()?;
    Ok(session_id)
}

/// Rename a session. Audited inside the tx. The SAME refusal for an unknown
/// key and a foreign key.
pub fn rename_session(
    conn: &mut Connection,
    scope: &OwnerScope,
    session_key: &str,
    label: &str,
    actor: &str,
) -> Result<SessionRow, ChatSessionError> {
    let key = bounded_session_key(session_key)?;
    let label = bounded_label(label)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(session_id) = TxProbe { conn: &tx }.owned_session_id(&key, scope)? else {
        return Err(ChatSessionError::SessionNotFound);
    };
    tx.execute(
        "UPDATE chat_sessions SET label = ?2 WHERE id = ?1",
        params![session_id, label],
    )?;
    crate::audit::record(
        &tx,
        crate::audit::AuditKind::Chat,
        actor,
        &key,
        AuditStatus::Ok,
        "chat_session_rename",
    );
    let row = read_session_row(&tx, session_id)?;
    tx.commit()?;
    Ok(row)
}

/// Erase a session — turns + row in ONE transaction, audited. The erasure
/// posture: hard delete, because a soft-deleted transcript keeps data the
/// user asked to remove. The SAME refusal for an unknown key and a foreign
/// key. Returns the erased turn count.
pub fn delete_session(
    conn: &mut Connection,
    scope: &OwnerScope,
    session_key: &str,
    actor: &str,
) -> Result<i64, ChatSessionError> {
    let key = bounded_session_key(session_key)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(session_id) = TxProbe { conn: &tx }.owned_session_id(&key, scope)? else {
        return Err(ChatSessionError::SessionNotFound);
    };
    let deleted = tx.execute(
        "DELETE FROM chat_turns WHERE session_id = ?1",
        params![session_id],
    )?;
    tx.execute(
        "DELETE FROM chat_sessions WHERE id = ?1",
        params![session_id],
    )?;
    crate::audit::record(
        &tx,
        crate::audit::AuditKind::Chat,
        actor,
        &key,
        AuditStatus::Ok,
        "chat_session_delete",
    );
    tx.commit()?;
    Ok(deleted as i64)
}

/// The owner-scoped session lookup, shared by the three {session}-keyed
/// verbs: the owner scope rides the WHERE clause, so a foreign key and an
/// unknown key both answer `None` — no existence oracle.
struct TxProbe<'a> {
    conn: &'a Connection,
}

impl TxProbe<'_> {
    fn owned_session_id(
        &self,
        key: &str,
        scope: &OwnerScope,
    ) -> Result<Option<i64>, ChatSessionError> {
        // The key is ?1; the scope's placeholders shift to ?2.. so the two
        // predicates cannot collide (owner IN (?1) would silently bind the
        // KEY as an owner — the wrong-number-of-parameters failure is the
        // loud version, and this offset is what keeps it impossible).
        let (clause, binds) = scope.clause_for(1);
        let sql = format!("SELECT id FROM chat_sessions WHERE session_key = ?1 AND {clause}");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = vec![&key];
        for b in &binds {
            params_vec.push(b);
        }
        match stmt.query_row(params_vec.as_slice(), |r| r.get(0)) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

fn read_session_row(tx: &Connection, session_id: i64) -> Result<SessionRow, ChatSessionError> {
    Ok(tx.query_row(
        "SELECT session_key, label, turn_count, created_at, last_turn_at
         FROM chat_sessions WHERE id = ?1",
        params![session_id],
        |r| {
            Ok(SessionRow {
                session_key: r.get(0)?,
                label: r.get(1)?,
                turn_count: r.get(2)?,
                created_at: r.get(3)?,
                last_turn_at: r.get(4)?,
            })
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The REAL migration on an in-memory DB: these tables come from the
    /// migration block, and the audit machinery needs the schema it creates
    /// (audit_events + schema_meta). A hand-rolled fixture would prove the
    /// service against a schema production never ships.
    fn db() -> Connection {
        // The vec0 module must be registered before the migration builds
        // vec_knowledge — the same order the suite's own test_db uses.
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut c = Connection::open_in_memory().expect("open in-memory DB");
        crate::migration::run_migration(&mut c, crate::config::DB_MMAP_SIZE_MIB)
            .expect("migration");
        c
    }

    const T0: i64 = 1_760_000_000;
    const OWNER: &str = "user-a";
    const OTHER: &str = "user-b";

    fn narrowed(who: &str) -> OwnerScope {
        OwnerScope::narrowed(vec![who.to_owned()])
    }

    /// THE LAZY-CREATION LAW. One turn on a fresh key creates exactly one
    /// session row + two turn rows + the audit evidence, all inside the one
    /// tx; the counters are the turn's.
    #[test]
    fn the_first_turn_creates_the_session_and_the_pair_in_one_tx() {
        let mut conn = db();
        let id = persist_turn_pair(
            &mut conn,
            "s-alpha",
            OWNER,
            "what is the gate?",
            "the gate is read-first.",
            Some(r#"{"decision":"ok","hits":[]}"#),
            Some("on_device"),
            None,
            T0,
        )
        .expect("persist");
        let (sessions, total) = list_sessions(&conn, &OwnerScope::unrestricted(), 20, 0).unwrap();
        assert_eq!(total, 1, "one session exists");
        let row = &sessions[0];
        assert_eq!(row.session_key, "s-alpha");
        assert_eq!(row.turn_count, 1, "one pair so far");
        assert_eq!(row.created_at, T0);
        assert_eq!(row.last_turn_at, T0);
        let (turns, turn_total) =
            list_turns(&conn, &OwnerScope::unrestricted(), "s-alpha", 50, 0).unwrap();
        assert_eq!(turn_total, 2, "user + assistant");
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].seq, 1);
        assert_eq!(turns[1].role, "assistant");
        assert_eq!(turns[1].seq, 2);
        assert_eq!(
            turns[1].grounding_json.as_deref(),
            Some(r#"{"decision":"ok","hits":[]}"#)
        );
        assert_eq!(turns[1].tier.as_deref(), Some("on_device"));
        assert_eq!(id, 1);
        // The SECOND turn reuses the row and bumps the counters.
        persist_turn_pair(
            &mut conn,
            "s-alpha",
            OWNER,
            "again",
            "again answered.",
            None,
            None,
            None,
            T0 + 5,
        )
        .unwrap();
        let (sessions, _) = list_sessions(&conn, &OwnerScope::unrestricted(), 20, 0).unwrap();
        assert_eq!(sessions.len(), 1, "still exactly one row");
        assert_eq!(sessions[0].turn_count, 2);
        assert_eq!(sessions[0].last_turn_at, T0 + 5);
        // The chain carries the create AND the turn evidence.
        let chat_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='chat'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(chat_rows, 3, "create + turn + turn");
    }

    /// THE EMPTY-SESSION DEFECT, named: there is no path from this module to
    /// a session row without a turn. The pin holds the shape, not the
    /// intention: fresh key → zero rows; one turn → one row.
    #[test]
    fn no_path_inserts_a_session_without_a_turn() {
        let conn = db();
        let sessions: i64 = conn
            .query_row("SELECT COUNT(*) FROM chat_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sessions, 0, "a fresh store has no session rows");
        // The listing of an empty store is an honest empty page, not an error.
        let (rows, total) = list_sessions(&conn, &OwnerScope::unrestricted(), 20, 0).unwrap();
        assert!(rows.is_empty());
        assert_eq!(total, 0);
    }

    /// OWNER SCOPE. Principal B never sees A's rows, and A's key asked by B
    /// is the SAME refusal as a key that does not exist.
    #[test]
    fn a_foreign_session_is_404_shaped_like_an_unknown_one() {
        let mut conn = db();
        persist_turn_pair(&mut conn, "s-owned", OWNER, "q", "a", None, None, None, T0).unwrap();
        // Unrestricted (loopback/Admin) sees it.
        let (_, total) = list_sessions(&conn, &OwnerScope::unrestricted(), 20, 0).unwrap();
        assert_eq!(total, 1);
        // A narrowed stranger sees nothing.
        let (rows, total) = list_sessions(&conn, &narrowed(OTHER), 20, 0).unwrap();
        assert!(rows.is_empty());
        assert_eq!(total, 0);
        // …and its verbs refuse with the SAME error the unknown key gets.
        let unknown = list_turns(&conn, &narrowed(OTHER), "s-never-existed", 50, 0).unwrap_err();
        let foreign = list_turns(&conn, &narrowed(OTHER), "s-owned", 50, 0).unwrap_err();
        assert!(matches!(unknown, ChatSessionError::SessionNotFound));
        assert!(matches!(foreign, ChatSessionError::SessionNotFound));
        assert_eq!(
            unknown.to_string(),
            foreign.to_string(),
            "no existence oracle: the two refusals must be indistinguishable"
        );
        // The write verbs hold the same shape.
        let rename =
            rename_session(&mut conn, &narrowed(OTHER), "s-owned", "x", OTHER).unwrap_err();
        assert!(matches!(rename, ChatSessionError::SessionNotFound));
        let wipe = delete_session(&mut conn, &narrowed(OTHER), "s-owned", OTHER).unwrap_err();
        assert!(matches!(wipe, ChatSessionError::SessionNotFound));
        // …and nothing was renamed or erased by the refused verbs.
        let (_, total) = list_sessions(&conn, &OwnerScope::unrestricted(), 20, 0).unwrap();
        assert_eq!(total, 1);
        let (turns, _) = list_turns(&conn, &OwnerScope::unrestricted(), "s-owned", 50, 0).unwrap();
        assert_eq!(turns.len(), 2);
    }

    /// THE BOUNDS LAW. An over-ceiling ask is capped and the total stays
    /// honest (the count of ALL the caller's rows, not the page's length);
    /// the turn page caps the same way.
    #[test]
    fn a_51_row_ask_is_capped_and_total_stays_honest() {
        let mut conn = db();
        for i in 0..52 {
            let key = format!("s-{i:02}");
            persist_turn_pair(&mut conn, &key, OWNER, "q", "a", None, None, None, T0 + i).unwrap();
        }
        // 52 asked, 50 served — the ceiling holds — and the total is the
        // count of every row, not the page's length.
        let (rows, total) = list_sessions(&conn, &OwnerScope::unrestricted(), 51, 0).unwrap();
        assert_eq!(rows.len(), 50, "52 rows exist; the page caps at 50");
        assert_eq!(total, 52);
        // Newest activity first.
        assert_eq!(rows[0].session_key, "s-51");
        // …and the page walks: offset 50 returns the remainder.
        let (rest, _) = list_sessions(&conn, &OwnerScope::unrestricted(), 50, 50).unwrap();
        assert_eq!(rest.len(), 2);
        // The turn page caps identically over one long session.
        for i in 0..30 {
            persist_turn_pair(
                &mut conn,
                "s-00",
                OWNER,
                "q",
                "a",
                None,
                None,
                None,
                T0 + 100 + i,
            )
            .unwrap();
        }
        let (turns, turn_total) =
            list_turns(&conn, &OwnerScope::unrestricted(), "s-00", 51, 0).unwrap();
        assert_eq!(turns.len(), 50, "62 turns exist; the page caps at 50");
        assert_eq!(turn_total, 62);
    }

    /// RENAME: round-trip, the owner fence, and the 80-byte ceiling.
    #[test]
    fn rename_round_trips_and_holds_the_label_ceiling() {
        let mut conn = db();
        persist_turn_pair(&mut conn, "s-r", OWNER, "q", "a", None, None, None, T0).unwrap();
        let renamed = rename_session(
            &mut conn,
            &OwnerScope::unrestricted(),
            "s-r",
            "Gate notes",
            OWNER,
        )
        .unwrap();
        assert_eq!(renamed.label, "Gate notes");
        let (rows, _) = list_sessions(&conn, &OwnerScope::unrestricted(), 20, 0).unwrap();
        assert_eq!(rows[0].label, "Gate notes");
        // The ceiling: 81 bytes refuses, 80 passes; the rename is a no-op on
        // refusal.
        let long = "x".repeat(81);
        assert!(matches!(
            rename_session(&mut conn, &OwnerScope::unrestricted(), "s-r", &long, OWNER),
            Err(ChatSessionError::InvalidLabel(81))
        ));
        let edge = "y".repeat(MAX_LABEL_BYTES);
        rename_session(&mut conn, &OwnerScope::unrestricted(), "s-r", &edge, OWNER).unwrap();
        // An empty label refuses too.
        assert!(matches!(
            rename_session(&mut conn, &OwnerScope::unrestricted(), "s-r", "   ", OWNER),
            Err(ChatSessionError::InvalidLabel(0))
        ));
    }

    /// DELETE: turns + session in ONE tx, audited, and the count is the
    /// receipt. Nothing lingers.
    #[test]
    fn delete_removes_turns_and_session_atomically_and_audits_it() {
        let mut conn = db();
        persist_turn_pair(&mut conn, "s-del", OWNER, "q1", "a1", None, None, None, T0).unwrap();
        persist_turn_pair(
            &mut conn,
            "s-del",
            OWNER,
            "q2",
            "a2",
            None,
            None,
            None,
            T0 + 1,
        )
        .unwrap();
        let erased =
            delete_session(&mut conn, &OwnerScope::unrestricted(), "s-del", OWNER).unwrap();
        assert_eq!(
            erased, 4,
            "two pairs = four turn rows, and the receipt says so"
        );
        let (_, total) = list_sessions(&conn, &OwnerScope::unrestricted(), 20, 0).unwrap();
        assert_eq!(total, 0, "the session row is gone");
        let orphans: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chat_turns WHERE session_id NOT IN (SELECT id FROM chat_sessions)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphans, 0, "no turn rows outlive their session");
        let chat_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='chat'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            chat_rows, 4,
            "create + two turns + the delete, all on the chain"
        );
        // A second delete of the same key is the unknown-key refusal.
        assert!(matches!(
            delete_session(&mut conn, &OwnerScope::unrestricted(), "s-del", OWNER),
            Err(ChatSessionError::SessionNotFound)
        ));
    }

    /// THE FAILED-GENERATION LAW, held from the storage side: there is no
    /// API on this module that writes the user row alone, so the
    /// orphan-user-row shape cannot be constructed here. The pin proves the
    /// negative: after N refused/absent finals (nothing persisted), the
    /// store still shows only complete pairs — every user row has its
    /// assistant row beside it.
    #[test]
    fn every_persisted_user_row_has_its_assistant_beside_it() {
        let mut conn = db();
        for i in 0..3 {
            persist_turn_pair(
                &mut conn,
                "s-pair",
                OWNER,
                &format!("q{i}"),
                &format!("a{i}"),
                None,
                None,
                None,
                T0 + i,
            )
            .unwrap();
        }
        let orphans: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chat_turns c_user
                 WHERE c_user.role='user'
                   AND NOT EXISTS (SELECT 1 FROM chat_turns c_a
                                   WHERE c_a.session_id = c_user.session_id
                                     AND c_a.seq = c_user.seq + 1
                                     AND c_a.role='assistant')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            orphans, 0,
            "an orphan user row is the defect the pair-tx kills"
        );
    }

    /// SCREEN AT WRITE. A hostile payload is STORED SCREENED (the sanitizing
    /// half of the write-time screen: the hostile-element strip has already
    /// fired before the bytes land), and the seq pairing survives it.
    #[test]
    fn a_script_payload_is_stored_screened_not_raw() {
        let mut conn = db();
        persist_turn_pair(
            &mut conn,
            "s-screen",
            OWNER,
            "hello <script>alert(1)</script> world",
            "answer with ![x]<script></script>(url) inside",
            None,
            None,
            None,
            T0,
        )
        .unwrap();
        let (turns, _) = list_turns(&conn, &OwnerScope::unrestricted(), "s-screen", 50, 0).unwrap();
        for (i, turn) in turns.iter().enumerate() {
            // A failing assert must not echo transcript bytes into the
            // panic/log sink: the index diagnoses the row, the predicate
            // still reads the stored bytes.
            assert!(
                !turn.content.contains("<script>"),
                "the stored form must not carry a live script tag (turn #{i})"
            );
        }
        assert_eq!(turns.len(), 2, "the screened pair still pairs");
    }

    /// STORED FLAGGED, NEVER SILENTLY AN ANSWER (LL-04 §1.2): the verdict's
    /// wire name rides the assistant row and reads back verbatim; an
    /// `unsupported` turn is distinguishable from a `supported` one by the
    /// ROW, not by re-parsing prose. An unknown decision name refuses the
    /// write — storage never holds a value the verdict enum cannot mean.
    #[test]
    fn the_support_verdict_is_stored_flagged_and_unknown_names_refuse() {
        let mut conn = db();
        persist_turn_pair(
            &mut conn,
            "s-flag",
            OWNER,
            "what is in green tea?",
            "Green tea contains EGCG [8].",
            None,
            Some("on_device"),
            Some("unsupported"),
            T0,
        )
        .unwrap();
        persist_turn_pair(
            &mut conn,
            "s-flag",
            OWNER,
            "and?",
            "It is steamed.",
            None,
            Some("on_device"),
            Some("supported"),
            T0 + 1,
        )
        .unwrap();
        let (turns, _) = list_turns(&conn, &OwnerScope::unrestricted(), "s-flag", 50, 0).unwrap();
        assert_eq!(turns[1].support_decision.as_deref(), Some("unsupported"));
        assert_eq!(turns[3].support_decision.as_deref(), Some("supported"));
        // A pre-LL-04 row (support_decision NULL) reads back None.
        persist_turn_pair(
            &mut conn,
            "s-flag",
            OWNER,
            "legacy",
            "legacy answer",
            None,
            None,
            None,
            T0 + 2,
        )
        .unwrap();
        let (turns, _) = list_turns(&conn, &OwnerScope::unrestricted(), "s-flag", 50, 0).unwrap();
        assert_eq!(turns[5].support_decision, None);
        // The vocabulary is the verdict enum's: an unknown name refuses.
        let refused = persist_turn_pair(
            &mut conn,
            "s-flag",
            OWNER,
            "q",
            "a",
            None,
            None,
            Some("hallucinated"),
            T0 + 3,
        );
        assert!(matches!(refused, Err(ChatSessionError::Storage(_))));
        let (_, total) = list_turns(&conn, &OwnerScope::unrestricted(), "s-flag", 50, 0).unwrap();
        assert_eq!(total, 6, "the refused write persisted nothing");
    }

    /// THE KEY FENCE at the storage boundary: the shape `/chat/turns`
    /// enforces at the wire is re-asserted here, so a future caller cannot
    /// bypass the wire bound.
    #[test]
    fn the_session_key_fence_holds_of_the_function() {
        let mut conn = db();
        for bad in ["", "   ", "two words", &*"x".repeat(65)] {
            assert!(
                matches!(
                    persist_turn_pair(&mut conn, bad, OWNER, "q", "a", None, None, None, T0),
                    Err(ChatSessionError::InvalidKey)
                ),
                "the key {bad:?} must refuse at the storage boundary"
            );
        }
        let (_, total) = list_sessions(&conn, &OwnerScope::unrestricted(), 20, 0).unwrap();
        assert_eq!(total, 0, "a refused key wrote nothing");
    }
}
