//! Versioned GDL journal contract. No lease recovery: only an owner that has
//! finished an exchange may explicitly pause. A crash retains its claim.
//! rusqlite rollback-on-drop verified via Context7 2026-09-17 (upstream docs).

use super::{GdlCase, GdlOutcome, GdlPhase, LoopError, MAX_PHASE_ATTEMPTS, session_log};
use crate::workflow::{state, tx::WorkflowTx};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub(super) fn persist_error(e: impl std::fmt::Display) -> LoopError {
    LoopError::Persist(e.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Checkpoint {
    version: u32,
    run: i64,
    domain: String,
    policy: String,
    pub episode: String,
    pub revision: i64,
    pub case: GdlCase,
    pub next_phase: Option<GdlPhase>,
    /// Completed failed attempts in next_phase, not the in-flight attempt.
    pub attempt: u32,
    pub errors: Vec<String>,
    pub phases: u32,
    pub exchange: Option<i64>,
    pub terminal: Option<GdlOutcome>,
    status: String,
}

impl Checkpoint {
    /// The run's domain, for the post-gate additive law checks that need
    /// it (the T7 must-miss catalog) — the domain is checkpoint state
    /// (bound to the run row), deliberately not case state.
    pub(super) fn case_domain(&self) -> &str {
        &self.domain
    }

    fn key(&self) -> String {
        format!("control:gdl:{}", self.revision)
    }

    fn json(&self) -> Result<String, LoopError> {
        serde_json::to_string(self).map_err(persist_error)
    }

    fn store(&self, conn: &Connection) -> Result<(), LoopError> {
        session_log::append(
            conn,
            self.run,
            "control:gdl",
            &self.json()?,
            &self.key(),
            chrono::Utc::now().timestamp(),
        )
        .map_err(persist_error)?;
        Ok(())
    }

    fn validate(&self) -> Result<(), LoopError> {
        let phase_count = self.next_phase.map_or(7, |p| p as u32);
        if self.version != 1
            || self.revision < 1
            || self.episode.is_empty()
            || self.episode.len() > 128
            || self.attempt > MAX_PHASE_ATTEMPTS
            || (self.terminal.is_none() && self.attempt == MAX_PHASE_ATTEMPTS)
            || self.phases > 7
            || self.errors.len() > 64
            || self.errors.iter().any(|s| s.len() > 16_384)
            || (self.attempt == 0) != self.errors.is_empty()
            || (self.terminal.is_none()
                && (self.next_phase.is_none() || self.phases != phase_count))
            || self.exchange.is_some_and(|id| id <= 0)
            || self.status
                != if matches!(self.terminal, Some(GdlOutcome::Resolved { .. })) {
                    "resolved"
                } else {
                    "active"
                }
        {
            return Err(persist_error("malformed or unsupported GDL checkpoint"));
        }
        let terminal_matches = match &self.terminal {
            Some(GdlOutcome::Resolved {
                phases,
                verify,
                capture,
            }) => {
                *phases == 7
                    && self.phases == 7
                    && self.next_phase.is_none()
                    && self.case.phase == GdlPhase::Handoff
                    && verify.pass
                    && self.case.verify.as_ref() == Some(verify)
                    && self.case.capture.as_ref() == Some(capture)
            }
            Some(GdlOutcome::VerifyFailed { at, bundle }) => {
                *at == GdlPhase::Verify
                    && self.case.phase == *at
                    && self.case.verify.as_ref().is_some_and(|v| !v.pass)
                    && *bundle == self.case.escalation_bundle()
            }
            Some(GdlOutcome::Escalated { at, bundle }) => {
                self.next_phase == Some(*at)
                    && self.case.phase == *at
                    && *bundle == self.case.escalation_bundle()
            }
            Some(GdlOutcome::Routed { at, reason }) => {
                self.next_phase == Some(*at)
                    && self.attempt == MAX_PHASE_ATTEMPTS
                    && *reason
                        == format!(
                            "gate exhausted after {} attempts: {}",
                            self.attempt,
                            self.errors.join("; ")
                        )
            }
            Some(GdlOutcome::Capped { at, reason }) => {
                self.next_phase == Some(*at) && matches!(reason.as_str(), "turn_cap" | "budget")
            }
            Some(GdlOutcome::Canceled) => self.next_phase.is_some(),
            None => true,
        };
        if !terminal_matches {
            return Err(persist_error("GDL terminal/case mismatch"));
        }
        Ok(())
    }

    pub fn verify(&self, conn: &Connection, owner: &str) -> Result<(), LoopError> {
        session_log::verify_owner(conn, self.run, owner).map_err(persist_error)?;
        let row = read_run(conn, self.run)?;
        self.bind(&row)?;
        let stored = session_log::exact(conn, self.run, &self.key())
            .map_err(persist_error)?
            .ok_or_else(|| persist_error("GDL checkpoint absent"))?;
        if stored.kind != "control:gdl" || stored.payload_json != self.json()? {
            return Err(persist_error("GDL checkpoint changed"));
        }
        self.verify_link(conn)
    }

    fn verify_link(&self, conn: &Connection) -> Result<(), LoopError> {
        if self.revision == 1 {
            if self.exchange.is_some()
                || self.terminal.is_some()
                || self.phases != 0
                || self.attempt != 0
                || self.case != GdlCase::fresh(&self.case.ticket)
                || self.next_phase != Some(GdlPhase::Intake)
            {
                return Err(persist_error("invalid initial GDL checkpoint"));
            }
            return Ok(());
        }
        if self.revision > 23 {
            return Err(persist_error(
                "GDL checkpoint revision exceeds episode bound",
            ));
        }
        let row = session_log::exact(conn, self.run, &format!("{}:gate", self.key()))
            .map_err(persist_error)?
            .ok_or_else(|| persist_error("GDL gate missing"))?;
        let gate: GateRecord = serde_json::from_str(&row.payload_json)
            .map_err(|_| persist_error("corrupt GDL gate"))?;
        if row.kind != "gdl_gate"
            || gate.version != 1
            || gate.episode != self.episode
            || gate.revision != self.revision - 1
            || gate.exchange != self.exchange
            || gate.terminal != self.terminal
        {
            return Err(persist_error("GDL gate/checkpoint mismatch"));
        }
        let prior_row =
            session_log::exact(conn, self.run, &format!("control:gdl:{}", gate.revision))
                .map_err(persist_error)?
                .ok_or_else(|| persist_error("GDL prior checkpoint missing"))?;
        let prior: Checkpoint = serde_json::from_str(&prior_row.payload_json)
            .map_err(|_| persist_error("corrupt prior GDL checkpoint"))?;
        prior.validate()?;
        if prior_row.kind != "control:gdl"
            || prior.revision != gate.revision
            || prior.run != self.run
            || prior.domain != self.domain
            || prior.policy != self.policy
            || prior.episode != self.episode
            || prior.terminal.is_some()
            || prior.next_phase != Some(gate.phase)
            || gate.attempt != prior.attempt + 1
        {
            return Err(persist_error("GDL transition predecessor mismatch"));
        }
        prior.verify_link(conn)?;
        let mut expected = prior.case.clone();
        expected.phase = gate.phase;
        if let Some(artifact) = &gate.artifact {
            super::apply(&mut expected, gate.phase, artifact);
        }
        let accepted = gate.verdict == "pass";
        let next_phase = if accepted {
            GdlPhase::ALL.get((prior.phases + 1) as usize).copied()
        } else {
            prior.next_phase
        };
        let (attempt, errors) = if accepted {
            (0, Vec::new())
        } else if gate.verdict == "fail" {
            (gate.attempt, gate.errors.clone())
        } else {
            (prior.attempt, prior.errors.clone())
        };
        if expected != self.case
            || accepted != gate.artifact.is_some()
            || self.phases != prior.phases + u32::from(accepted)
            || self.next_phase != next_phase
            || self.attempt != attempt
            || self.errors != errors
        {
            return Err(persist_error("GDL gate artifact/state mismatch"));
        }
        gate.verify_receipt(conn, self.run, row.seq)?;
        Ok(())
    }

    fn bind(&self, row: &Run) -> Result<(), LoopError> {
        self.validate()?;
        if row.kind != "troubleshoot"
            || row.domain != self.domain
            || row.status != self.status
            || row.revision != self.revision
            || row.json != serde_json::to_string(&self.case).map_err(persist_error)?
        {
            return Err(persist_error(
                "GDL state/status/revision binding changed; explicit new episode refused",
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GateRecord {
    version: u32,
    phase: GdlPhase,
    verdict: String,
    attempt: u32,
    errors: Vec<String>,
    episode: String,
    exchange: Option<i64>,
    revision: i64,
    artifact: Option<String>,
    terminal: Option<GdlOutcome>,
}

// Local decoder for the existing identity journal contract; no SDK change.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExchangeEnd {
    version: u32,
    outcome: super::RunOutcome,
    assistant_key: Option<String>,
}

impl GateRecord {
    fn verify_receipt(&self, conn: &Connection, run: i64, gate_seq: i64) -> Result<(), LoopError> {
        use super::RunOutcome;
        let Some(id) = self.exchange else {
            if self.verdict == "route"
                && self.artifact.is_none()
                && matches!(&self.terminal, Some(GdlOutcome::Escalated { at, .. }) if *at == self.phase)
                && self.errors.len() == 1
                && self.errors[0]
                    == format!(
                        "authority: phase {} is owned by a higher tier — escalating with the bundle",
                        self.phase.as_str()
                    )
            {
                return Ok(());
            }
            return Err(persist_error("GDL transition lacks exchange receipt"));
        };
        let start = session_log::exact(
            conn,
            run,
            &format!(
                "control:exchange:gdl:{}:phase:{}:attempt:{}",
                self.episode,
                self.phase.as_str(),
                self.attempt
            ),
        )
        .map_err(persist_error)?
        .ok_or_else(|| persist_error("GDL exchange start missing"))?;
        let done = session_log::exact(conn, run, &format!("control:exchange_done:{id}"))
            .map_err(persist_error)?
            .ok_or_else(|| persist_error("GDL exchange receipt missing"))?;
        let end: ExchangeEnd = serde_json::from_str(&done.payload_json)
            .map_err(|_| persist_error("corrupt GDL exchange receipt"))?;
        // The driver's fingerprint is "v2:<policy>:v2:<input>" (hash_text is
        // versioned); validate shape, not a pinned digest.
        let parts: Vec<&str> = start.payload_json.split(':').collect();
        let fingerprint_ok = parts.len() == 4
            && parts[0] == "v2"
            && parts[2] == "v2"
            && parts[1].len() == 64
            && parts[3].len() == 64
            && parts[1].bytes().all(|b| b.is_ascii_hexdigit())
            && parts[3].bytes().all(|b| b.is_ascii_hexdigit());
        if id <= 0
            || start.seq != id
            || start.kind != "control:exchange"
            || done.kind != "control:exchange_done"
            || end.version != 1
            || done.seq <= id
            || done.seq >= gate_seq
            || !fingerprint_ok
        {
            return Err(persist_error("GDL exchange identity/kind mismatch"));
        }
        let turns = match &end.outcome {
            RunOutcome::Completed { turns, .. }
            | RunOutcome::TurnCapReached { turns, .. }
            | RunOutcome::BudgetExceeded { turns, .. } => Some(*turns),
            RunOutcome::Canceled => None,
        };
        let text = if let Some(key) = end.assistant_key {
            let turn = key
                .strip_prefix(&format!("run{run}:a{id}:t"))
                .and_then(|s| s.parse::<u32>().ok());
            if turn.is_none_or(|t| t == 0) || turns.is_some_and(|t| Some(t) != turn) {
                return Err(persist_error("GDL receipt assistant identity mismatch"));
            }
            let assistant = session_log::exact(conn, run, &key)
                .map_err(persist_error)?
                .ok_or_else(|| persist_error("GDL receipt assistant absent"))?;
            let value: serde_json::Value =
                serde_json::from_str(&assistant.payload_json).map_err(persist_error)?;
            if assistant.kind != "assistant" || assistant.seq <= id || assistant.seq >= done.seq {
                return Err(persist_error("GDL receipt assistant kind/order mismatch"));
            }
            value
                .get("text")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| persist_error("GDL receipt assistant text absent"))?
                .to_string()
        } else if turns.is_none_or(|t| t == 0) {
            String::new()
        } else {
            return Err(persist_error("GDL receipt assistant reference absent"));
        };
        let compatible = match (&end.outcome, &self.terminal) {
            (RunOutcome::Canceled, Some(GdlOutcome::Canceled)) => true,
            (RunOutcome::TurnCapReached { .. }, Some(GdlOutcome::Capped { reason, .. })) => {
                reason == "turn_cap"
            }
            (RunOutcome::BudgetExceeded { .. }, Some(GdlOutcome::Capped { reason, .. })) => {
                reason == "budget"
            }
            (
                RunOutcome::Completed { .. },
                None
                | Some(
                    GdlOutcome::Resolved { .. }
                    | GdlOutcome::Routed { .. }
                    | GdlOutcome::Escalated { .. }
                    | GdlOutcome::VerifyFailed { .. },
                ),
            ) => true,
            _ => false,
        };
        if !compatible {
            return Err(persist_error("GDL outcome/exchange mismatch"));
        }
        if let Some(artifact) = &self.artifact {
            let (_, normalized) = super::accept_ungated(self.phase, &text);
            if self.verdict != "pass" || normalized.as_ref() != Some(artifact) {
                return Err(persist_error("GDL gate/receipt artifact mismatch"));
            }
        } else if !matches!(self.verdict.as_str(), "route" | "fail") {
            return Err(persist_error("invalid GDL receipt gate verdict"));
        }
        Ok(())
    }
}

fn checked_audit(
    conn: &Connection,
    cp: &Checkpoint,
    status: crate::audit::AuditStatus,
    detail: &str,
) -> Result<(), LoopError> {
    crate::audit::record_tenant(
        conn,
        crate::audit::AuditKind::Workflow,
        crate::workflow::ACTOR,
        &cp.key(),
        status,
        detail,
        &cp.domain,
    )
    .ok_or_else(|| persist_error("GDL checked audit insertion failed"))?;
    Ok(())
}

struct Run {
    json: String,
    revision: i64,
    kind: String,
    status: String,
    domain: String,
}

fn read_run(conn: &Connection, run: i64) -> Result<Run, LoopError> {
    conn.query_row(
        "SELECT CASE WHEN length(CAST(state_json AS BLOB)) <= ?2 THEN state_json ELSE NULL END,
         state_revision, kind, status, domain FROM workflow_runs WHERE id=?1",
        params![run, session_log::PAYLOAD_CAP_BYTES as i64],
        |r| {
            Ok(Run {
                json: r.get(0)?,
                revision: r.get(1)?,
                kind: r.get(2)?,
                status: r.get(3)?,
                domain: r.get(4)?,
            })
        },
    )
    .map_err(persist_error)
}

/// Validation and acquisition share BEGIN IMMEDIATE. In particular the fresh
/// sentinel is checked BEFORE our own claim creates session history.
pub(super) fn admit(
    conn: &mut Connection,
    run: i64,
    ticket: &str,
    policy: &str,
    owner: &str,
) -> Result<Checkpoint, LoopError> {
    let mut tx = WorkflowTx::begin(conn).map_err(persist_error)?;
    let row = read_run(tx.tx(), run)?;
    let latest: Option<String> = tx.tx().query_row(
        "SELECT idempotency_key FROM agent_session_events WHERE run_id=?1 AND kind='control:gdl' ORDER BY seq DESC LIMIT 1",
        [run], |r| r.get(0),
    ).optional().map_err(persist_error)?;
    let cp = if let Some(key) = latest {
        let event = session_log::exact(tx.tx(), run, &key)
            .map_err(persist_error)?
            .ok_or_else(|| persist_error("GDL checkpoint absent"))?;
        let cp: Checkpoint = serde_json::from_str(&event.payload_json)
            .map_err(|_| persist_error("corrupt GDL checkpoint"))?;
        cp.bind(&row)?;
        cp.verify_link(tx.tx())?;
        if cp.run != run || cp.key() != key || cp.case.ticket != ticket || cp.policy != policy {
            return Err(persist_error("GDL invocation binding mismatch"));
        }
        if cp.terminal.is_none() {
            let pause = session_log::exact(tx.tx(), run, &format!("{}:pause", cp.key()))
                .map_err(persist_error)?
                .ok_or_else(|| {
                    persist_error("GDL interrupted; explicit quiescent pause required")
                })?;
            let last: i64 = tx
                .tx()
                .query_row(
                    "SELECT MAX(seq) FROM agent_session_events WHERE run_id=?1",
                    [run],
                    |r| r.get(0),
                )
                .map_err(persist_error)?;
            if pause.kind != "control:gdl_pause"
                || pause.payload_json != cp.json()?
                || last != pause.seq + 1
            {
                return Err(persist_error(
                    "GDL pause changed or intervening session work",
                ));
            }
        }
        session_log::acquire(tx.tx(), run, owner).map_err(persist_error)?;
        cp
    } else {
        let history: bool = tx.tx().query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE run_id=?1) OR EXISTS(SELECT 1 FROM agent_session_events WHERE run_id=?1)",
            [run], |r| r.get(0),
        ).map_err(persist_error)?;
        if row.kind != "troubleshoot"
            || row.status != "active"
            || row.revision != 0
            || row.json.trim() != "{}"
            || history
        {
            return Err(persist_error("corrupt state or ambiguous legacy GDL run"));
        }
        session_log::acquire(tx.tx(), run, owner).map_err(persist_error)?;
        let cp = Checkpoint {
            version: 1,
            run,
            domain: row.domain,
            policy: policy.into(),
            episode: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            case: GdlCase::fresh(ticket),
            next_phase: Some(GdlPhase::Intake),
            attempt: 0,
            errors: Vec::new(),
            phases: 0,
            exchange: None,
            terminal: None,
            status: "active".into(),
        };
        state::cas_update(
            tx.tx(),
            run,
            0,
            &serde_json::to_string(&cp.case).map_err(persist_error)?,
            "active",
            chrono::Utc::now().timestamp(),
        )
        .map_err(persist_error)?;
        cp.store(tx.tx())?;
        checked_audit(
            tx.tx(),
            &cp,
            crate::audit::AuditStatus::Ok,
            "gdl_checkpoint_sealed",
        )?;
        cp
    };
    if cp.terminal.is_some() {
        session_log::release(tx.tx(), run, owner).map_err(persist_error)?;
    }
    tx.commit().map_err(persist_error)?;
    Ok(cp)
}

pub(super) struct Transition {
    pub phase: GdlPhase,
    pub attempt: u32,
    pub verdict: &'static str,
    pub errors: Vec<String>,
    pub artifact: Option<String>,
    pub exchange: Option<i64>,
    pub terminal: Option<GdlOutcome>,
}

/// Called only after the exchange has settled, never across an external await.
pub(super) fn advance(
    conn: &mut Connection,
    old: &Checkpoint,
    owner: &str,
    change: Transition,
) -> Result<Checkpoint, LoopError> {
    let result = advance_tx(conn, old, owner, change);
    if result.is_err()
        && let Err(denial_error) = record_denial(conn, old, owner)
    {
        tracing::error!(error = ?denial_error, "GDL denial persistence failed; original checkpoint error retained");
    }
    result
}

fn record_denial(conn: &mut Connection, old: &Checkpoint, owner: &str) -> Result<(), LoopError> {
    // The failed transaction has dropped. append's normal Ok audit is
    // best-effort; the additional Denied row is REQUIRED in this transaction.
    let mut tx = WorkflowTx::begin(conn).map_err(persist_error)?;
    session_log::append(
        tx.tx(),
        old.run,
        "control:gdl_denied",
        &serde_json::json!({
            "version": 1, "episode": old.episode, "revision": old.revision,
            "reason": "checkpoint_refused"
        })
        .to_string(),
        &format!("{}:denied:{owner}", old.key()),
        chrono::Utc::now().timestamp(),
    )
    .map_err(persist_error)?;
    checked_audit(
        tx.tx(),
        old,
        crate::audit::AuditStatus::Denied,
        "gdl_checkpoint_refused",
    )?;
    tx.commit().map_err(persist_error)?;
    Ok(())
}

fn advance_tx(
    conn: &mut Connection,
    old: &Checkpoint,
    owner: &str,
    change: Transition,
) -> Result<Checkpoint, LoopError> {
    let mut tx = WorkflowTx::begin(conn).map_err(persist_error)?;
    old.verify(tx.tx(), owner)?;
    session_log::check_idle(tx.tx(), old.run).map_err(persist_error)?;
    if old.terminal.is_some()
        || old.next_phase != Some(change.phase)
        || change.attempt != old.attempt + 1
    {
        return Err(persist_error("invalid GDL transition"));
    }
    let mut next = old.clone();
    next.revision = old
        .revision
        .checked_add(1)
        .ok_or_else(|| persist_error("GDL revision overflow"))?;
    next.exchange = change.exchange;
    next.terminal = change.terminal.clone();
    next.case.phase = change.phase;
    if let Some(artifact) = &change.artifact {
        super::apply(&mut next.case, change.phase, artifact);
        write_phase(tx.tx(), old.run, change.phase, artifact)?;
        next.phases += 1;
        next.next_phase = GdlPhase::ALL.get(next.phases as usize).copied();
        next.attempt = 0;
        next.errors.clear();
    } else if change.verdict == "fail" {
        next.attempt = change.attempt;
        next.errors.clone_from(&change.errors);
    }
    if matches!(next.terminal, Some(GdlOutcome::Resolved { .. })) {
        next.status = "resolved".into();
    }
    next.validate()?;
    session_log::append(
        tx.tx(),
        old.run,
        "gdl_gate",
        &serde_json::to_string(&GateRecord {
            version: 1,
            phase: change.phase,
            verdict: change.verdict.into(),
            attempt: change.attempt,
            errors: change.errors,
            episode: old.episode.clone(),
            exchange: change.exchange,
            revision: old.revision,
            artifact: change.artifact,
            terminal: change.terminal,
        })
        .map_err(persist_error)?,
        &format!("{}:gate", next.key()),
        chrono::Utc::now().timestamp(),
    )
    .map_err(persist_error)?;
    state::cas_update(
        tx.tx(),
        old.run,
        old.revision,
        &serde_json::to_string(&next.case).map_err(persist_error)?,
        &next.status,
        chrono::Utc::now().timestamp(),
    )
    .map_err(persist_error)?;
    if matches!(next.terminal, Some(GdlOutcome::Resolved { .. })) {
        crate::workflow::proficiency::capture_proposals_on_resolve(
            &mut tx,
            old.run,
            &next.case,
            chrono::Utc::now().timestamp(),
        )
        .map_err(persist_error)?;
    }
    next.store(tx.tx())?;
    next.verify_link(tx.tx())?;
    checked_audit(
        tx.tx(),
        &next,
        crate::audit::AuditStatus::Ok,
        "gdl_checkpoint_sealed",
    )?;
    if next.terminal.is_some() {
        session_log::release(tx.tx(), old.run, owner).map_err(persist_error)?;
    }
    tx.commit().map_err(persist_error)?;
    Ok(next)
}

pub(super) fn pause(conn: &mut Connection, cp: &Checkpoint, owner: &str) -> Result<(), LoopError> {
    let mut tx = WorkflowTx::begin(conn).map_err(persist_error)?;
    cp.verify(tx.tx(), owner)?;
    session_log::check_idle(tx.tx(), cp.run).map_err(persist_error)?;
    if cp.terminal.is_some() {
        return Err(persist_error("terminal GDL cannot pause"));
    }
    let pending: bool = tx.tx().query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_session_events e WHERE e.run_id=?1 AND e.kind='control:exchange'
         AND NOT EXISTS(SELECT 1 FROM agent_session_events d WHERE d.run_id=e.run_id
         AND d.idempotency_key='control:exchange_done:' || e.seq AND d.kind='control:exchange_done'))",
        [cp.run], |r| r.get(0),
    ).map_err(persist_error)?;
    if pending {
        return Err(persist_error("unsettled exchange cannot pause"));
    }
    session_log::append(
        tx.tx(),
        cp.run,
        "control:gdl_pause",
        &cp.json()?,
        &format!("{}:pause", cp.key()),
        chrono::Utc::now().timestamp(),
    )
    .map_err(persist_error)?;
    session_log::release(tx.tx(), cp.run, owner).map_err(persist_error)?;
    tx.commit().map_err(persist_error)?;
    Ok(())
}

fn write_phase(
    conn: &Connection,
    run: i64,
    phase: GdlPhase,
    artifact: &str,
) -> Result<(), LoopError> {
    let name = phase.as_str();
    conn.execute(
        "INSERT INTO workflow_steps(run_id, phase, step_key, state_json) VALUES (?1, ?2, ?2, ?3)",
        params![run, name, artifact],
    )
    .map_err(persist_error)?;
    let parent = conn.last_insert_rowid();
    crate::workflow::audit_write(
        conn,
        run,
        &format!("step:{name}"),
        crate::audit::AuditStatus::Ok,
        &format!("gdl phase {name} passed"),
    );
    if phase == GdlPhase::Act {
        let rows: super::ActArtifact = serde_json::from_str(artifact).map_err(persist_error)?;
        for row in rows.rows {
            let key = format!("act-{}", row.order);
            conn.execute("INSERT INTO workflow_steps(run_id, phase, step_key, state_json, parent_step_id) VALUES (?1, 'act', ?2, ?3, ?4)", params![run, key, serde_json::to_string(&row).map_err(persist_error)?, parent]).map_err(persist_error)?;
            crate::workflow::audit_write(
                conn,
                run,
                &format!("step:{key}"),
                crate::audit::AuditStatus::Ok,
                &format!("test-log row {} persisted", row.order),
            );
        }
    }
    let batch = super::typed_evidence_for(phase, artifact, chrono::Utc::now().timestamp());
    if !batch.is_empty() {
        crate::workflow::evidence::record(conn, run, &batch, None)
            .map_err(|e| persist_error(format!("evidence: {e:?}")))?;
    }
    Ok(())
}
