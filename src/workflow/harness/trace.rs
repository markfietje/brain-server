//! The decision-run trace: the replayable artifact of one pipeline run
//! (digests, refs, per-stage records — the architecture's trace shape) and
//! its persistence. DIGESTS AND REFS ONLY: query-adjacent free text is
//! hashed (the normalized ask carries `query_digest`), raw evidence text
//! is unrepresentable one level up (the retrieval seam returns
//! reference-only hits), and the trace therefore binds immutable
//! references — input/context digests, pipeline version, config hash,
//! model digests, retrieval parameters, the compile-time environment
//! fingerprint.
//!
//! Persistence: ONE additive table (the recall-traces precedent — created
//! by the migration, never touched otherwise) plus the session-log kinds'
//! writer. The writer owns ONE BEGIN IMMEDIATE transition (the house
//! write discipline, `WorkflowTx::begin`): the trace row and every
//! session event commit together or not at all; the session-log appends
//! are exactly-once by idempotency key and their audit rows ride the
//! same transition. `pub(crate)`: no route reads or writes any of it —
//! the writer ships before its callers.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::config::{LoadedPipelineConfig, RetrievalParams, canonical_json};
use super::pipeline::{
    ActionLabel, ContextHit, DecisionRunRequest, DecisionRunResult, EscalationRecord,
    ModelRefRecord, SerdeDecisionOutput, StageRecord,
};
use super::{DECISION_OUTPUT_KIND, DECISION_REFUSAL_KIND, DECISION_RUN_KIND, PIPELINE_VERSION};
use crate::workflow::session_log;
use crate::workflow::tx::WorkflowTx;

/// The compile-time environment fingerprint: the kernel's package version
/// and the feature flags this binary was built with. No environment reads
/// at run time — `env!` and `cfg!` are compile-time.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct EnvFingerprint {
    pub(crate) kernel_version: String,
    pub(crate) feature_flags: Vec<String>,
}

impl EnvFingerprint {
    fn current() -> Self {
        let mut feature_flags = Vec::new();
        if cfg!(feature = "bench") {
            feature_flags.push("bench".to_string());
        }
        if cfg!(feature = "otel") {
            feature_flags.push("otel".to_string());
        }
        if cfg!(feature = "compliance-pack") {
            feature_flags.push("compliance-pack".to_string());
        }
        if cfg!(feature = "migrate") {
            feature_flags.push("migrate".to_string());
        }
        feature_flags.sort();
        EnvFingerprint {
            kernel_version: env!("CARGO_PKG_VERSION").to_string(),
            feature_flags,
        }
    }
}

/// The run's outcome face inside the trace: what was proposed and whether
/// it escalated — the typed escalation record's durable home (the harness
/// engine writes no other state).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct TraceOutcome {
    pub(crate) action: ActionLabel,
    pub(crate) escalation: Option<EscalationRecord>,
    pub(crate) output: Option<SerdeDecisionOutput>,
}

/// The decision-run trace — the architecture's shape: run identity, mode,
/// the config hash, model refs, the input commitment, the context refs
/// (ids + digests + tiers), the recorded retrieval parameters, the
/// environment fingerprint, and one record per declared stage.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct DecisionRunTrace {
    pub(crate) run_id: i64,
    pub(crate) pipeline_version: String,
    pub(crate) mode: String,
    pub(crate) config_hash: String,
    pub(crate) model_refs: Vec<ModelRefRecord>,
    pub(crate) input_digest: String,
    pub(crate) context_refs: Vec<ContextHit>,
    pub(crate) retrieval_params: RetrievalParams,
    pub(crate) env_fingerprint: EnvFingerprint,
    pub(crate) stages: Vec<StageRecord>,
    pub(crate) outcome: TraceOutcome,
}

/// Build the trace of one completed run. The mode is the request's own
/// recorded vocabulary (deterministic | exploratory).
pub(crate) fn build_decision_run_trace(
    loaded: &LoadedPipelineConfig,
    req: &DecisionRunRequest,
    result: &DecisionRunResult,
) -> DecisionRunTrace {
    DecisionRunTrace {
        run_id: result.run_id,
        pipeline_version: PIPELINE_VERSION.to_string(),
        mode: req.mode.as_str().to_string(),
        config_hash: loaded.config_hash.clone(),
        model_refs: vec![result.model.clone()],
        input_digest: result.input_digest.clone(),
        context_refs: result.context_hits.clone(),
        retrieval_params: loaded.config.retrieval,
        env_fingerprint: EnvFingerprint::current(),
        stages: result.records.clone(),
        outcome: TraceOutcome {
            action: result.action,
            escalation: result.escalation.clone(),
            output: result.output.clone(),
        },
    }
}

/// The writer's receipt: the trace row (and whether THIS call created it —
/// a replayed persist is a no-op receipt), plus the session-log seqs the
/// batch produced (a replay returns the ORIGINAL seqs, the append's own
/// law).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TracePersistReceipt {
    pub(crate) trace_row_id: i64,
    pub(crate) trace_created: bool,
    pub(crate) session_seqs: Vec<i64>,
}

fn refused(msg: String) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(msg)
}

/// Persist one decision-run trace: the `decision_run_traces` row AND the
/// session-log batch under the caller's run id — `decision_run` first (the
/// run's logical start; replay order mirrors execution order), then one
/// `decision_output`/`decision_refusal` per stage outcome. Exactly-once:
/// the table row by content (`run_id` + `trace_json` — a deterministic
/// re-run of the same trace is the same artifact), the session events by
/// idempotency key. All rows commit in ONE BEGIN IMMEDIATE transition the
/// writer owns. `now` is the row's creation instant (time as argument).
pub(crate) fn persist_decision_run_trace(
    conn: &mut Connection,
    trace: &DecisionRunTrace,
    now: i64,
) -> rusqlite::Result<TracePersistReceipt> {
    let trace_json = canonical_json(trace);
    // The house write discipline: ONE BEGIN IMMEDIATE transition the
    // writer owns — the trace row and every session event commit together
    // (WorkflowTx rolls back on any error drop).
    let mut wtx = WorkflowTx::begin(conn)?;

    if let Some(existing) = wtx
        .tx()
        .query_row(
            "SELECT id FROM decision_run_traces WHERE run_id = ?1 AND trace_json = ?2",
            rusqlite::params![trace.run_id, trace_json],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(TracePersistReceipt {
            trace_row_id: existing,
            trace_created: false,
            session_seqs: Vec::new(),
        });
    }

    wtx.tx().execute(
        "INSERT INTO decision_run_traces(run_id, mode, pipeline_version, config_hash, trace_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            trace.run_id,
            trace.mode,
            trace.pipeline_version,
            trace.config_hash,
            trace_json,
            now
        ],
    )?;
    let trace_row_id = wtx.tx().last_insert_rowid();

    // The key prefix is the trace's content identity: config + input
    // commitment. Two different decision runs under one run id coexist;
    // a replayed persist of the SAME trace hits the same keys and stays
    // exactly-once (the append's own no-op law).
    let key = |tail: &str| {
        format!(
            "decision:{}:{}:{}:{}",
            trace.run_id, trace.config_hash, trace.input_digest, tail
        )
    };

    let mut session_seqs = Vec::new();
    let run_payload = serde_json::json!({
        "mode": trace.mode,
        "pipeline_version": trace.pipeline_version,
        "config_hash": trace.config_hash,
        "input_digest": trace.input_digest,
    })
    .to_string();
    let (_, seq) = session_log::append(
        wtx.tx(),
        trace.run_id,
        DECISION_RUN_KIND,
        &run_payload,
        &key("run"),
        now,
    )?;
    session_seqs.push(seq);

    for (i, record) in trace.stages.iter().enumerate() {
        let payload = serde_json::json!({
            "stage": record.stage.as_str(),
            "algorithm": record.algorithm,
            "outputs_digest": record.outputs_digest,
            "trust_tiers_seen": record.trust_tiers_seen,
            "duration_ns": record.duration_ns,
        })
        .to_string();
        let (_, seq) = session_log::append(
            wtx.tx(),
            trace.run_id,
            DECISION_OUTPUT_KIND,
            &payload,
            &key(&format!("stage:{}:{}", i, record.stage.as_str())),
            now,
        )?;
        session_seqs.push(seq);
    }

    if let Some(escalation) = &trace.outcome.escalation {
        let payload = serde_json::json!({
            "stage": escalation.stage.as_str(),
            "reason": escalation.reason,
            "detail": escalation.detail,
        })
        .to_string();
        let (_, seq) = session_log::append(
            wtx.tx(),
            trace.run_id,
            DECISION_REFUSAL_KIND,
            &payload,
            &key(&format!("refusal:{}", escalation.stage.as_str())),
            now,
        )?;
        session_seqs.push(seq);
    }

    wtx.commit()?;
    Ok(TracePersistReceipt {
        trace_row_id,
        trace_created: true,
        session_seqs,
    })
}

/// The pipeline_version the stamp columns carry (the module's compile-time
/// constant, re-exported for the tests).
pub(crate) fn pipeline_version() -> &'static str {
    PIPELINE_VERSION
}

#[cfg(test)]
mod tests {
    use super::super::config::{DecisionPipelineConfig, RetrievalParams};
    use super::super::pipeline::{
        AskKind, AskQuestion, ContextRetriever, DeclaredHit, DeclaredListRetriever,
        RetrievalRefusal, run_decision_pipeline,
    };
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::harness::models::rules::RulesModel;
    use crate::workflow::session_log;
    use brain_engine_sdk::decision::{RunMode, TrustTier};
    use rusqlite::Connection;

    const DIGEST_A: &str = "cc0000000000000000000000000000000000000000000000000000000000000c";
    const QUERY: &str = "the raw query text must never reach the trace";

    const TABLE_JSON: &str = r#"{
      "model_id": "rules-reference",
      "model_version": "1.0.0",
      "rules": [
        { "question_id": "needs_human", "min_evidence": 1, "min_tier": "vetted",
          "output": { "Choice": { "options": ["act", "reject"], "label": "act" } } }
      ]
    }"#;

    fn config_json(model_digest: &str) -> String {
        format!(
            r#"{{
              "config_schema": "harness.pipeline/v1",
              "pipeline_id": "trace-line",
              "stages": ["normalize","retrieve_context","candidate_generation","decision_model","rules_policy","threshold","action_escalation"],
              "retrieval": {{"rrf_k": 60, "limit": 10, "leg": "vector"}},
              "model": {{"key": "rules:rules-reference", "digest": "{model_digest}"}},
              "thresholds": {{
                "act_labels": ["act"],
                "reject_labels": ["reject"],
                "score_act_at_or_above": 50,
                "score_reject_at_or_below": 10,
                "noul_act_when": true,
                "fallback": "approve"
              }}
            }}"#
        )
    }

    fn stepping_clock() -> impl Fn() -> i64 {
        let cell = std::cell::Cell::new(5_000);
        move || {
            let now = cell.get();
            cell.set(now + 100);
            now
        }
    }

    struct DownRetriever;
    impl ContextRetriever for DownRetriever {
        fn retrieve(
            &self,
            _query: &str,
            _params: &RetrievalParams,
        ) -> Result<Vec<ContextHit>, RetrievalRefusal> {
            Err(RetrievalRefusal::Unavailable)
        }
        fn algorithm(&self) -> &str {
            "test.down"
        }
    }

    fn test_db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    /// The storage law, end to end: the trace row + the R26 session-log
    /// kinds round-trip through the REAL session-log append/replay under
    /// the caller's run id; a replayed persist is a no-op receipt; the
    /// trace_json carries digests and refs and NEVER the raw query text;
    /// the environment fingerprint is compile-time truth.
    #[test]
    fn decision_run_trace_storage_is_additive_and_hash_lawful() {
        let model = RulesModel::from_canonical_json(TABLE_JSON).unwrap();
        let loaded = DecisionPipelineConfig::load(&config_json(model.digest())).unwrap();
        let retr = DeclaredListRetriever {
            vector: vec![DeclaredHit {
                id: 91,
                digest: DIGEST_A.to_string(),
                tier: TrustTier::Governed,
                untrusted: false,
                flagged: false,
            }],
            fts: Vec::new(),
            graph: Vec::new(),
        };
        let req = super::super::pipeline::DecisionRunRequest {
            run_id: 501,
            mode: RunMode::Deterministic,
            role_scope: vec!["operator".into()],
            created_at: 1_800_000_500,
            request_id: "req-trace".into(),
            question: Some(AskQuestion {
                id: "needs_human".into(),
                kind: AskKind::Choice,
            }),
            question_ids: vec!["needs_human".into()],
            query: QUERY,
        };
        let result = run_decision_pipeline(&loaded, &req, &model, &retr, &stepping_clock());
        assert_eq!(result.action, super::super::pipeline::ActionLabel::Act);
        let trace = build_decision_run_trace(&loaded, &req, &result);

        // The trace law: digests and refs only — the raw query text is
        // absent from the serialized artifact, and the input commitment
        // rides as its digest.
        let trace_json = canonical_json(&trace);
        assert!(
            !trace_json.contains(QUERY),
            "query-adjacent free text is hashed, never stored raw"
        );
        assert!(trace_json.contains(&result.input_digest));
        assert_eq!(trace.pipeline_version, pipeline_version());
        assert_eq!(
            trace.env_fingerprint.kernel_version,
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(trace.mode, "deterministic");
        assert_eq!(trace.stages.len(), result.records.len());

        // Persist: one row, one session batch, all-or-nothing.
        let mut conn = test_db();
        let receipt = persist_decision_run_trace(&mut conn, &trace, 1_800_000_600).unwrap();
        assert!(receipt.trace_created);
        let row_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decision_run_traces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(row_count, 1, "exactly one trace row");

        // The R26 kinds round-trip through the REAL session-log replay,
        // in execution order: run first, then one row per stage, then the
        // (absent here — the run acted) refusal.
        let replayed = session_log::replay(&conn, 501, session_log::REPLAY_CAP).unwrap();
        let kinds: Vec<&str> = replayed.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(kinds[0], DECISION_RUN_KIND);
        assert_eq!(
            kinds.len(),
            1 + trace.stages.len(),
            "one decision_run + one row per stage"
        );
        for kind in &kinds[1..] {
            assert_eq!(*kind, DECISION_OUTPUT_KIND);
        }
        assert_eq!(receipt.session_seqs.len(), kinds.len());
        // The exactly-once law: a replayed persist is a no-op receipt and
        // adds neither rows nor events.
        let again = persist_decision_run_trace(&mut conn, &trace, 1_800_000_700).unwrap();
        assert!(!again.trace_created);
        assert!(again.session_seqs.is_empty());
        assert_eq!(again.trace_row_id, receipt.trace_row_id);
        let (rows_after, events_after): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM decision_run_traces), \
                        (SELECT COUNT(*) FROM agent_session_events WHERE run_id = 501)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(rows_after, 1);
        assert_eq!(events_after, kinds.len() as i64);

        // An escalated run writes the refusal kind with the closed reason.
        let refused_req = super::super::pipeline::DecisionRunRequest {
            query: "the seam is down for this ask",
            ..req
        };
        let refused = run_decision_pipeline(
            &loaded,
            &refused_req,
            &model,
            &DownRetriever,
            &stepping_clock(),
        );
        let refused_trace = build_decision_run_trace(&loaded, &refused_req, &refused);
        assert_eq!(
            refused.action,
            super::super::pipeline::ActionLabel::Escalate
        );
        let receipt = persist_decision_run_trace(&mut conn, &refused_trace, 1_800_000_800).unwrap();
        assert!(receipt.trace_created);
        let replayed = session_log::replay(&conn, 501, session_log::REPLAY_CAP).unwrap();
        let refusal_row = replayed
            .iter()
            .find(|r| r.kind == DECISION_REFUSAL_KIND)
            .expect("the refusal event round-trips");
        let payload: serde_json::Value = serde_json::from_str(&refusal_row.payload_json).unwrap();
        assert_eq!(payload["stage"], "retrieve_context");
        assert_eq!(payload["reason"], "retrieval_unavailable");

        // Additive proof: the whole surface lives in its own table + event
        // kinds; the mode/pipeline_version/config_hash columns carry the
        // bounded-query face.
        let (mode, version, hash): (String, String, String) = conn
            .query_row(
                "SELECT mode, pipeline_version, config_hash FROM decision_run_traces WHERE id = ?1",
                rusqlite::params![receipt.trace_row_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(mode, "deterministic");
        assert_eq!(version, pipeline_version());
        assert_eq!(hash, refused_trace.config_hash);
    }
}
