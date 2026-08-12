//! v1.14.0 "Gate" — HTTP handlers for write-back gating (M1), decay + GDPR
//! lifecycle (M2). The pure logic lives in `src/gate.rs`; this module does the
//! HTTP + transaction wiring, reusing the existing ingest / consolidate /
//! sources machinery instead of re-implementing it.
//!
//! Routes:
//!   POST /ingest/proposal     — queue a scored candidate; no `knowledge` row.
//!   GET  /proposals?status=   — the human review queue.
//!   POST /proposals/{id}/approve[?supersedes=<id>] — promote (one tx).
//!   POST /proposals/{id}/reject — reject (audited, never deleted).
//!   GET  /decayed             — operator review of decayed chunks.
//!   GET  /export[?include_pii_map=true] — portable JSON export (GDPR).
//!   POST /purge               — hard, explicit, audited deletion (GDPR).
//!
//! Human-in-the-loop: nothing here auto-promotes, auto-decays-away, or
//! auto-deletes. The human decides. Zero tokens, no LLM, no background worker.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use crate::handlers::auth::{OptCapability, OptPrincipal};
use crate::handlers::{HandlerError, MAX_QUERY, MAX_SOURCE_PROMPT};

/// v1.20.2 D3: cap on `/export` row count. The export buffers every row into
/// memory then serializes; on a multi-GB DB this OOMs. We refuse above this
/// threshold + document the per-domain snapshot path. ponytail: a true
/// streaming encoder is a v2.x change; this guard prevents the OOM today.
pub const MAX_EXPORT_ROWS: i64 = 200_000;
use crate::AppState;

/// Max proposals returned per review page. Bounded so a runaway queue can't
/// unbounded a response.
const MAX_PROPOSALS: usize = 200;
/// Max ids accepted by a single `/purge` call. Explicit-only deletion must be
/// deliberate; a huge batch is a footgun.
const MAX_PURGE_IDS: usize = 1000;

/// `POST /ingest/proposal`
#[derive(Debug, Deserialize)]
pub struct ProposalRequest {
    pub content: String,
    /// memory_kind vocabulary (fact/procedure/step/decision/episodic).
    #[serde(default = "default_fact")]
    pub kind: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub authority: Option<f32>,
    #[serde(default)]
    pub observed_at: Option<i64>,
    #[serde(default)]
    pub domain: Option<String>,
    /// v1.20.1 "Shield" M2: the caller-provided prompt that fed this capture.
    /// Lets a reviewer context-check the proposal and lets the queue surface
    /// re-run the injection screen against the sourcing text.
    #[serde(default)]
    pub source_prompt: Option<String>,
}

fn default_fact() -> String {
    "fact".to_string()
}

#[derive(Debug, Serialize)]
pub struct ProposalResponse {
    pub id: i64,
    pub status: &'static str,
    pub novelty: f32,
    pub conflict_with: Option<i64>,
    pub salience: f32,
}

/// `POST /ingest/proposal` — queue a scored candidate. No `knowledge` row is
/// created; `/recall` cannot see it until a human approves.
/// v1.20.7 "Telemetry" (M1): emits a `gate.propose` span under `--features otel`
/// carrying proposal_id + screen_verdict + principal + domain (no content body).
#[cfg_attr(
    feature = "otel",
    tracing::instrument(
        name = "gate.propose",
        skip_all,
        fields(
            proposal_id = tracing::field::Empty,
            screen_verdict = tracing::field::Empty,
            principal = tracing::field::Empty,
            domain = tracing::field::Empty
        )
    )
)]
pub async fn ingest_proposal(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<ProposalRequest>,
) -> Result<Json<ProposalResponse>, HandlerError> {
    let domain = req
        .domain
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| "global".to_string());
    super::authorize(&principal.0, crate::auth::Action::Write, "", &domain)?;
    let content = req.content.trim().to_string();
    if content.is_empty() {
        return Err(HandlerError::bad_request(
            "empty_content",
            "content is required",
        ));
    }
    if content.chars().count() > MAX_QUERY {
        return Err(HandlerError::bad_request(
            "content_too_long",
            format!("content exceeds {MAX_QUERY} chars"),
        ));
    }
    // v1.20.3 "Classify" M1 (G5): run the injection screen on the proposal
    // content. `Reject` → 400, never persisted (the review queue only ever
    // sees `clean`/`quarantine`); `Quarantine` → stored + badged so the
    // reviewer sees the flag before approving a capture whose own text was
    // instruction-bearing. The badge is recomputed deterministically at read
    // time (list_proposals), so no schema change is needed.
    let screen_res = crate::screen::screen(&content, "");
    if screen_res == crate::screen::ScreenResult::Reject {
        return Err(HandlerError::bad_request(
            "input_rejected",
            "input contains suspicious patterns",
        ));
    }
    // v1.20.2 F1: bound + injection-screen `source_prompt`. The plugin caps at
    // 2000 client-side; the server enforces its own bound so a malicious caller
    // can't persist a 1 MiB prompt. If the screen trips, the screened form is
    // still stored (the reviewer needs to see WHY the capture tripped) — but a
    // warning is attached so a reviewer doesn't blindly approve a capture whose
    // own trigger text was instruction-bearing.
    if let Some(p) = req.source_prompt.as_deref() {
        if p.len() > MAX_SOURCE_PROMPT {
            return Err(HandlerError::bad_request(
                "source_prompt_too_long",
                format!("source_prompt exceeds {MAX_SOURCE_PROMPT} bytes"),
            ));
        }
    }
    if !crate::procedural::MemoryKind::from_str(&req.kind).is_valid_for_gate() {
        return Err(HandlerError::bad_request(
            "invalid_kind",
            format!("unknown memory_kind '{}'", req.kind),
        ));
    }

    let pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;
    let model = Arc::clone(&state.model);
    let content_for_task = content.clone();

    let resp = tokio::task::spawn_blocking(move || -> Result<ProposalResponse, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        // Deterministic scoring (M1): novelty via vec0 KNN, conflict via the
        // consolidate machinery, salience via the length/entity heuristic.
        let embedding = match model
            .encode(std::slice::from_ref(&content_for_task))
            .into_iter()
            .next()
        {
            Some(e) => e,
            None => {
                return Err(HandlerError::internal("embedding generation failed"));
            }
        };
        let novelty = crate::gate::novelty(&conn, &embedding).unwrap_or(1.0); // first memory / no index → max novelty
        let conflict_with = find_conflict(&conn, &content_for_task);
        let entity_count = crate::linker::extract_vocabulary(&content_for_task, &[])
            .entities
            .len();
        let salience = crate::gate::salience(&content_for_task, entity_count);
        let now = chrono::Utc::now().timestamp();

        let id: i64 = conn
            .query_row(
                "INSERT INTO proposals(kind, content, source, authority, observed_at,
                                   novelty, conflict_with, salience, created_at, source_prompt)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             RETURNING id",
                rusqlite::params![
                    req.kind,
                    content_for_task,
                    req.source,
                    req.authority,
                    req.observed_at,
                    novelty,
                    conflict_with,
                    salience,
                    now,
                    req.source_prompt
                        .as_deref()
                        .map(crate::gate::screen_source_prompt)
                ],
                |r| r.get(0),
            )
            .map_err(|e| HandlerError::internal(format!("proposal insert failed: {e}")))?;

        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Ingest,
            "api",
            &format!("proposal:{id}"),
            crate::audit::AuditStatus::Ok,
            "proposal_pending",
        );

        Ok(ProposalResponse {
            id,
            status: "pending",
            novelty,
            conflict_with,
            salience,
        })
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    #[cfg(feature = "otel")]
    {
        let span = tracing::Span::current();
        span.record("proposal_id", resp.id);
        span.record(
            "screen_verdict",
            crate::otel::screen_verdict_span(screen_res),
        );
        span.record("principal", super::recall::principal_label(&principal.0));
        span.record("domain", domain.clone());
    }

    // v1.20.8 "Signal": alert the console a candidate is awaiting review
    // (pending), and separately when the injection screen tripped (screen).
    // Payloads are signals, never content/PII.
    let now = chrono::Utc::now().timestamp();
    crate::alert::publish(
        &state,
        crate::alert::ALERT_KIND_PENDING,
        json!({
            "proposal_id": resp.id,
            "screen_verdict": crate::screen::screen_verdict_label(screen_res),
            "created_at": now,
            "expires_at": now + crate::config::proposal_ttl_secs(),
        }),
    );
    if screen_res != crate::screen::ScreenResult::Clean {
        crate::alert::publish(
            &state,
            crate::alert::ALERT_KIND_SCREEN,
            json!({ "verdict": crate::screen::screen_verdict_label(screen_res) }),
        );
    }

    Ok(Json(resp))
}

/// Find a live chunk whose subject conflicts with the candidate content. Reuses
/// [`crate::consolidate::find_subject_conflicts`]'s signal: a candidate that
/// contradicts an existing current claim is flagged in the review queue so the
/// human sees the conflict, not a silent overwrite.
fn find_conflict(conn: &rusqlite::Connection, content: &str) -> Option<i64> {
    // Cheap exact-subject pre-check before the O(n²) pairwise scan: only run
    // the full conflict scan when the candidate's subject appears somewhere.
    let subject = content
        .lines()
        .next()
        .unwrap_or(content)
        .chars()
        .take(120)
        .collect::<String>();
    let mut stmt = conn
        .prepare("SELECT id FROM knowledge WHERE (title IS NOT NULL AND title = ?1) OR (heading_path IS NOT NULL AND heading_path = ?1) AND valid_to IS NULL LIMIT 1")
        .ok()?;
    let matched: Option<i64> = stmt
        .query_row(rusqlite::params![subject], |r| r.get(0))
        .ok();
    drop(stmt);
    // Full pairwise conflict scan only when we have a subject-anchored hit.
    if matched.is_some() {
        if let Ok(pairs) = crate::consolidate::find_subject_conflicts(conn) {
            return pairs.into_iter().map(|p| p.from_chunk).next();
        }
    }
    None
}

#[derive(Debug, Deserialize)]
pub struct ProposalListQuery {
    #[serde(default = "default_pending")]
    pub status: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn default_pending() -> String {
    "pending".to_string()
}

#[derive(Debug, Serialize)]
pub struct ProposalView {
    pub id: i64,
    pub kind: String,
    pub content: String,
    pub source: Option<String>,
    pub source_prompt: Option<String>,
    pub authority: Option<f32>,
    pub novelty: f32,
    pub conflict_with: Option<i64>,
    pub salience: f32,
    pub created_at: i64,
    /// v1.20.3 "Classify" M1 (G5): the injection-screen verdict for `content`,
    /// recomputed deterministically at read time. `clean` or `quarantine` only
    /// (`reject` is never persisted — see `ingest_proposal`).
    pub screen_verdict: String,
    /// v1.20.14 "Steer" M1: unix ts of the last content rewrite, `None` if the
    /// pending proposal was never edited. Keys the review badge + read-time view.
    pub edited_at: Option<i64>,
    /// v1.20.15 "Clock" M1: when this proposal ages out of the review window
    /// (unix ts), derived server-side as `created_at + TTL`. The review queue
    /// ticks against this absolute deadline, so an operator override of
    /// `BRAIN_PROPOSAL_TTL_SECS` is authoritative (no client TTL guess).
    pub expires_at: i64,
    /// v1.20.15 "Clock" M1: the SLA band boundaries (secs of remaining life), a
    /// mirror of the alert watcher's `ALERT_WARN_SECS`/`ALERT_CRITICAL_SECS` so
    /// the client colors its countdown from the same thresholds as the server.
    pub warn_secs: i64,
    pub critical_secs: i64,
}

/// v1.20.15 "Clock": the review deadline + SLA bands, shared by every
/// `ProposalView` construction site so the countdown is one definition.
pub fn proposal_deadline(created_at: i64) -> (i64, i64, i64) {
    (
        created_at + crate::config::proposal_ttl_secs(),
        crate::config::ALERT_WARN_SECS,
        crate::config::ALERT_CRITICAL_SECS,
    )
}

/// `GET /proposals?status=pending&limit=` — the human review queue. Each item
/// carries its score components so the decision is evidence-based.
pub async fn list_proposals(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<ProposalListQuery>,
) -> Result<Json<Vec<ProposalView>>, HandlerError> {
    let domain = "global";
    super::authorize(&principal.0, crate::auth::Action::Read, "", domain)?;
    let status = q.status.trim().to_string();
    if !matches!(status.as_str(), "pending" | "approved" | "rejected") {
        return Err(HandlerError::bad_request(
            "invalid_status",
            "status must be pending|approved|rejected",
        ));
    }
    let limit = q.limit.unwrap_or(50).min(MAX_PROPOSALS);
    let pool = super::resolve_domain_pool(&state.registry, Some(domain))?;

    let rows = tokio::task::spawn_blocking(move || -> Result<Vec<ProposalView>, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, content, source, source_prompt, authority, novelty, conflict_with,
                        salience, created_at, edited_at
                 FROM proposals WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![status, limit as i64], |r| {
                let content: String = r.get(2)?;
                let created_at: i64 = r.get(9)?;
                let (expires_at, warn_secs, critical_secs) = proposal_deadline(created_at);
                Ok(ProposalView {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    screen_verdict: crate::screen::screen_verdict_label(crate::screen::screen(
                        &content, "",
                    ))
                    .to_string(),
                    content,
                    source: r.get(3)?,
                    source_prompt: r.get(4)?,
                    authority: r.get(5)?,
                    novelty: r.get(6)?,
                    conflict_with: r.get(7)?,
                    salience: r.get(8)?,
                    created_at,
                    edited_at: r.get(10)?,
                    expires_at,
                    warn_secs,
                    critical_secs,
                })
            })
            .map_err(|e| HandlerError::internal(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>();
        Ok(rows)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(rows))
}

/// v1.20.1 "Shield" M2 TTL: if the proposal is older than
/// [`crate::config::proposal_ttl_secs`], mark it rejected + audit
/// `proposal_expired` and return `false`. A stale auto-capture prompt's
/// context is unrecoverable, so the queue refuses to act on it (neither
/// approve nor reject — it's already beyond verification).
pub(crate) fn expire_if_stale(
    conn: &rusqlite::Connection,
    id: i64,
    created_at: i64,
) -> Result<bool, HandlerError> {
    let now = chrono::Utc::now().timestamp();
    if now - created_at <= crate::config::proposal_ttl_secs() {
        return Ok(true);
    }
    conn.execute(
        "UPDATE proposals SET status = 'rejected', decided_at = ?1
         WHERE id = ?2 AND status = 'pending'",
        rusqlite::params![now, id],
    )
    .map_err(|e| HandlerError::internal(e.to_string()))?;
    crate::audit::record(
        conn,
        crate::audit::AuditKind::Reconcile,
        "api",
        &format!("proposal:{id}"),
        crate::audit::AuditStatus::Ok,
        "proposal_expired",
    );
    Ok(false)
}

/// `POST /proposals/{id}/approve` — promote a candidate into long-term memory.
/// One transaction: creates the chunk (memory_kind, authority, observed_at),
/// marks the proposal approved + decided_at. With `?supersedes=<id>`, calls
/// [`crate::consolidate::resolve_supersession`] in the SAME transaction so
/// approving a conflicting fact atomically supersedes the old one.
/// v1.20.7 "Telemetry" (M1): emits a `gate.approve` span (proposal_id + outcome
/// + principal) under `--features otel`.
#[cfg_attr(
    feature = "otel",
    tracing::instrument(
        name = "gate.approve",
        skip_all,
        fields(
            proposal_id = tracing::field::Empty,
            outcome = tracing::field::Empty,
            principal = tracing::field::Empty
        )
    )
)]
pub async fn approve_proposal(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Query(q): Query<ApproveQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let domain = "global";
    super::authorize(&principal.0, crate::auth::Action::Write, "", domain)?;
    let pool = super::resolve_domain_pool(&state.registry, Some(domain))?;
    let model = Arc::clone(&state.model);

    // v1.20.7 "Telemetry" (M1): capture the actor label before `principal` is
    // moved into the blocking closure below (the closure promotes via
    // `principal_to_owner`), so the span can record it afterward.
    #[cfg(feature = "otel")]
    let principal_lbl = super::recall::principal_label(&principal.0);

    let res: Result<serde_json::Value, HandlerError> =
        tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;

        // v1.20.2 A4: run the TTL check + expiration audit on the raw autocommit
        // connection BEFORE opening the tx. Previously `expire_if_stale` was
        // called inside the tx, so its `proposal_expired` audit row rolled back
        // if anything between here and `tx.commit()` failed. Now the expiration
        // event lands independently + the re-check inside the tx catches a
        // concurrent state change.
        //
        // v1.20.2 A3: BEGIN IMMEDIATE so the SELECT-then-promote is serialized
        // against any concurrent approve/reject — eliminates the double-promote
        // race that was previously caught only by the content_hash UNIQUE index
        // (which surfaced as a generic 500 to the loser).
        let stale_created_at: Option<i64> = conn
            .query_row(
                "SELECT created_at FROM proposals WHERE id = ?1 AND status = 'pending'",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok();
        if let Some(created_at) = stale_created_at {
            if !expire_if_stale(&conn, id, created_at)? {
                return Err(HandlerError::bad_request(
                    "proposal_expired",
                    "proposal aged out of the review window (TTL), refused",
                ));
            }
        }

        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| HandlerError::internal(e.to_string()))?;

        // Load the pending proposal (re-checked inside the tx to catch a
        // concurrent state change since the autocommit expire check above).
        #[derive(Default)]
        struct ProposalRow {
            kind: String,
            content: String,
            source: Option<String>,
            authority: Option<f32>,
            observed_at: Option<i64>,
            created_at: i64,
        }
        let p: Option<ProposalRow> = tx
            .query_row(
                "SELECT kind, content, source, authority, observed_at, created_at
                 FROM proposals WHERE id = ?1 AND status = 'pending'",
                rusqlite::params![id],
                |r| {
                    Ok(ProposalRow {
                        kind: r.get(0)?,
                        content: r.get(1)?,
                        source: r.get(2)?,
                        authority: r.get(3)?,
                        observed_at: r.get(4)?,
                        created_at: r.get(5)?,
                    })
                },
            )
            .ok();
        let Some(p) = p else {
            return Err(HandlerError::not_found(format!(
                "no pending proposal with id {id}"
            )));
        };
        let (kind, content, source, authority, observed_at, _created_at) = (
            p.kind,
            p.content,
            p.source,
            p.authority,
            p.observed_at,
            p.created_at,
        );

        // Embed + insert the chunk through the same knowledge + vec0 path.
        let embedding = model
            .encode(std::slice::from_ref(&content))
            .into_iter()
            .next()
            .ok_or_else(|| HandlerError::internal("embedding generation failed"))?;
        let content_hash = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(content.as_bytes()));
        let source_kind = source.clone().unwrap_or_else(|| "manual".to_string());
        let assertion = "stated"; // promoted proposals are declarative by default
        let confidence = crate::gate::confidence(
            Some(source_kind.as_str()),
            false,
            assertion,
        );
        let now_utc = chrono::Utc::now().to_rfc3339();

        tx.execute(
            "INSERT INTO knowledge(content, title, source, content_hash, authority,
                                   observed_at, node_kind, assertion_kind, confidence, owner, origin)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                content,
                None::<String>,
                source_kind,
                content_hash,
                authority,
                observed_at.map(|o| o.to_string()),
                kind,
                assertion,
                confidence,
                principal_to_owner(&principal.0),
                crate::gate::origin_for_source(Some(&source_kind))
            ],
        )
        .map_err(|e| HandlerError::internal(format!("insert failed: {e}")))?;
        let chunk_id = tx.last_insert_rowid();

        // v1.13.6: strip reasoning traces at the ingest door (same as /add).
        tx.execute(
            "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
             VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), ?3, datetime('now'))",
            rusqlite::params![chunk_id, embedding.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>(), source_kind],
        )
        .map_err(|e| HandlerError::internal(format!("vec0 insert failed: {e}")))?;

        // Optional supersession in the same tx.
        if let Some(supersedes) = q.supersedes {
            if supersedes == chunk_id {
                return Err(HandlerError::bad_request(
                    "self_supersede",
                    "cannot supersede the chunk being created",
                ));
            }
            crate::consolidate::resolve_supersession(&tx, chunk_id, supersedes, &now_utc)
                .map_err(|e| HandlerError::internal(format!("supersession failed: {e}")))?;
        }

        // v1.20.2 A3: CAS the proposals row — `AND status = 'pending'` so a
        // concurrent approve/reject can't both succeed. Combined with the
        // IMMEDIATE tx above, this eliminates the double-promote race; the
        // UNIQUE content_hash index is the last-resort backstop.
        let n = tx
            .execute(
                "UPDATE proposals SET status = 'approved', decided_at = ?1
                 WHERE id = ?2 AND status = 'pending'",
                rusqlite::params![chrono::Utc::now().timestamp(), id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        if n == 0 {
            // A concurrent approve/reject won the race — abort cleanly.
            tx.rollback()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            return Err(HandlerError::conflict(format!(
                "proposal {id} was already decided by a concurrent action"
            )));
        }

        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;

        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            "api",
            &format!("proposal:{id}"),
            crate::audit::AuditStatus::Ok,
            "proposal_approved",
        );

        Ok(serde_json::json!({
            "proposal_id": id,
            "chunk_id": chunk_id,
            "status": "approved",
            "superseded": q.supersedes,
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?;

    #[cfg(feature = "otel")]
    {
        let span = tracing::Span::current();
        span.record("proposal_id", id);
        span.record("outcome", crate::otel::gate_outcome("approved", &res));
        span.record("principal", principal_lbl);
    }

    Ok(Json(res?))
}

#[derive(Debug, Default, Deserialize)]
pub struct ApproveQuery {
    #[serde(default)]
    pub supersedes: Option<i64>,
}

/// The owner string recorded on a chunk at ingest: the principal's subject when
/// a JWT principal exists, else NULL (loopback/opaque = unowned, the documented
/// legacy default). v1.17.1: now `pub` so the direct-ingest insert sites write it
/// (fixing the DSAR locate gap — a real DSAR could find nothing by subject).
pub fn principal_to_owner(p: &Option<crate::auth::Principal>) -> Option<String> {
    p.as_ref().map(|pr| pr.sub.clone())
}

/// v1.14.0 "Gate" M4: record-level access-scope filter for retrieval. In JWT
/// mode a principal may only see chunks whose `access_scope` is in their
/// allowed set (deny-by-default). The set is derived from the principal's
/// existing scopes: an `admin` scope sees everything; otherwise the principal
/// sees `private` (own) + `domain` + `team` scopes they hold. `None` (loopback/
/// opaque = no JWT) trusts localhost and sees everything (SECURITY.md posture).
pub fn scope_filter(p: &Option<crate::auth::Principal>) -> Option<Vec<String>> {
    match p {
        None => None, // loopback/opaque: trusts localhost
        Some(pr) => {
            if pr
                .scopes
                .iter()
                .any(|s| s.action == crate::auth::Action::Admin)
            {
                None // admin: unrestricted (standing trusted-reader group)
            } else {
                Some(vec![
                    "private".to_string(),
                    "domain".to_string(),
                    "team".to_string(),
                ])
            }
        }
    }
}

/// `POST /proposals/{id}/reject` — mark rejected + decided_at. Kept in the
/// audit trail (append-only, hash-only via `/audit`); never silently dropped,
/// never deleted.
/// v1.20.7 "Telemetry" (M1): emits a `gate.reject` span (proposal_id + outcome
/// + principal) under `--features otel`.
#[cfg_attr(
    feature = "otel",
    tracing::instrument(
        name = "gate.reject",
        skip_all,
        fields(
            proposal_id = tracing::field::Empty,
            outcome = tracing::field::Empty,
            principal = tracing::field::Empty
        )
    )
)]
pub async fn reject_proposal(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;

    let updated = tokio::task::spawn_blocking(move || -> Result<usize, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        // v1.20.1 M2: refuse to act on an expired proposal (audits + rejects it).
        let created_at: Option<i64> = conn
            .query_row(
                "SELECT created_at FROM proposals WHERE id = ?1 AND status = 'pending'",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok();
        if let Some(created_at) = created_at {
            if !expire_if_stale(&conn, id, created_at)? {
                return Err(HandlerError::bad_request(
                    "proposal_expired",
                    "proposal aged out of the review window (TTL), refused",
                ));
            }
        }
        let n = conn
            .execute(
                "UPDATE proposals SET status = 'rejected', decided_at = ?1
                 WHERE id = ?2 AND status = 'pending'",
                rusqlite::params![chrono::Utc::now().timestamp(), id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        if n > 0 {
            crate::audit::record(
                &conn,
                crate::audit::AuditKind::Reconcile,
                "api",
                &format!("proposal:{id}"),
                crate::audit::AuditStatus::Ok,
                "proposal_rejected",
            );
        }
        Ok(n)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    if updated == 0 {
        #[cfg(feature = "otel")]
        {
            let span = tracing::Span::current();
            span.record("outcome", "not_found");
            span.record("principal", super::recall::principal_label(&principal.0));
        }
        return Err(HandlerError::not_found(format!(
            "no pending proposal with id {id}"
        )));
    }
    #[cfg(feature = "otel")]
    {
        let span = tracing::Span::current();
        span.record("outcome", "rejected");
        span.record("principal", super::recall::principal_label(&principal.0));
    }
    Ok(Json(
        serde_json::json!({ "proposal_id": id, "status": "rejected" }),
    ))
}

/// `POST /proposals/{id}/edit` — rewrite a pending proposal's content and
/// re-score it deterministically (novelty / conflict / salience, the same path
/// as `ingest_proposal`). The injection screen still runs (`Reject` → 400;
/// `Quarantine` → allowed + stored, the read-time verdict badge recomputes it).
/// `edited_at` is stamped so the review badge survives a refresh; the audit
/// detail carries only the SHA-256 of the before + after content (never raw
/// text — consistent with the existing hash-only audit practice).
/// v1.20.7 "Telemetry" (M1): emits a `gate.edit` span under `--features otel`.
#[cfg_attr(
    feature = "otel",
    tracing::instrument(
        name = "gate.edit",
        skip_all,
        fields(
            proposal_id = tracing::field::Empty,
            outcome = tracing::field::Empty,
            principal = tracing::field::Empty
        )
    )
)]
pub async fn edit_proposal(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(req): Json<ProposalEditRequest>,
) -> Result<Json<ProposalView>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    let domain = "global";
    let content = req.content.trim().to_string();
    if content.is_empty() {
        return Err(HandlerError::bad_request(
            "empty_content",
            "content is required",
        ));
    }
    if content.chars().count() > MAX_QUERY {
        return Err(HandlerError::bad_request(
            "content_too_long",
            format!("content exceeds {MAX_QUERY} chars"),
        ));
    }
    let screen_res = crate::screen::screen(&content, "");
    if screen_res == crate::screen::ScreenResult::Reject {
        return Err(HandlerError::bad_request(
            "input_rejected",
            "input contains suspicious patterns",
        ));
    }
    let screen_label = crate::screen::screen_verdict_label(screen_res).to_string();

    #[cfg(feature = "otel")]
    let principal_lbl = super::recall::principal_label(&principal.0);

    let pool = super::resolve_domain_pool(&state.registry, Some(domain))?;
    let model = Arc::clone(&state.model);

    let res: Result<ProposalView, HandlerError> =
        tokio::task::spawn_blocking(move || -> Result<ProposalView, HandlerError> {
            let mut conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;

            // Same stale/expiry discipline as approve/reject (v1.20.2 A4):
            // the TTL check + expiration audit land on the raw autocommit conn
            // BEFORE the tx, then the tx re-checks `status='pending'`.
            let created_at: Option<i64> = conn
                .query_row(
                    "SELECT created_at FROM proposals WHERE id = ?1 AND status = 'pending'",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .ok();
            if let Some(ct) = created_at {
                if !expire_if_stale(&conn, id, ct)? {
                    return Err(HandlerError::bad_request(
                        "proposal_expired",
                        "proposal aged out of the review window (TTL), refused",
                    ));
                }
            }

            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| HandlerError::internal(e.to_string()))?;

            #[derive(Default)]
            struct Row {
                kind: String,
                content: String,
                source: Option<String>,
                source_prompt: Option<String>,
                authority: Option<f32>,
                created_at: i64,
            }
            let p: Option<Row> = tx
                .query_row(
                    "SELECT kind, content, source, source_prompt, authority, created_at
                     FROM proposals WHERE id = ?1 AND status = 'pending'",
                    rusqlite::params![id],
                    |r| {
                        Ok(Row {
                            kind: r.get(0)?,
                            content: r.get(1)?,
                            source: r.get(2)?,
                            source_prompt: r.get(3)?,
                            authority: r.get(4)?,
                            created_at: r.get(5)?,
                        })
                    },
                )
                .ok();
            let Some(p) = p else {
                return Err(HandlerError::not_found(format!(
                    "no pending proposal with id {id}"
                )));
            };
            let Row {
                kind,
                content: before,
                source,
                source_prompt,
                authority,
                created_at,
            } = p;

            // Re-score the edited content deterministically (the ingest path).
            let embedding = model
                .encode(std::slice::from_ref(&content))
                .into_iter()
                .next()
                .ok_or_else(|| HandlerError::internal("embedding generation failed"))?;
            let new_novelty = crate::gate::novelty(&tx, &embedding).unwrap_or(1.0);
            let new_conflict = find_conflict(&tx, &content);
            let entity_count = crate::linker::extract_vocabulary(&content, &[])
                .entities
                .len();
            let new_salience = crate::gate::salience(&content, entity_count);

            let now = chrono::Utc::now().timestamp();
            let n = tx
                .execute(
                    "UPDATE proposals SET content = ?1, novelty = ?2, salience = ?3,
                            conflict_with = ?4, edited_at = ?5
                     WHERE id = ?6 AND status = 'pending'",
                    rusqlite::params![content, new_novelty, new_salience, new_conflict, now, id],
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            if n == 0 {
                // A concurrent approve/reject won the race — abort cleanly.
                tx.rollback()
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                return Err(HandlerError::conflict(format!(
                    "proposal {id} was already decided by a concurrent action"
                )));
            }
            tx.commit()
                .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;

            // Audit: hashes only (SHA-256 of before + after content), never raw text.
            let detail = format!(
                "proposal:{id} {} {}",
                sha256_hex(&before),
                sha256_hex(&content)
            );
            crate::audit::record(
                &conn,
                crate::audit::AuditKind::Reconcile,
                "api",
                &format!("proposal:{id}"),
                crate::audit::AuditStatus::Ok,
                &detail,
            );

            let (expires_at, warn_secs, critical_secs) = proposal_deadline(created_at);
            Ok(ProposalView {
                id,
                kind,
                content,
                source,
                source_prompt,
                authority,
                novelty: new_novelty,
                conflict_with: new_conflict,
                salience: new_salience,
                created_at,
                screen_verdict: screen_label,
                edited_at: Some(now),
                expires_at,
                warn_secs,
                critical_secs,
            })
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?;

    #[cfg(feature = "otel")]
    {
        let span = tracing::Span::current();
        span.record("proposal_id", id);
        span.record("outcome", if res.is_ok() { "edited" } else { "error" });
        span.record("principal", principal_lbl);
    }

    Ok(Json(res?))
}

/// `POST /proposals/{id}/edit` request body.
#[derive(Debug, Deserialize)]
pub struct ProposalEditRequest {
    pub content: String,
}

/// v1.20.14 "Steer": hex SHA-256 of a string, for the edit audit detail (the
/// before/after hashes prove an edit happened without persisting the content).
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

// ── M2: decay + GDPR lifecycle ─────────────────────────────────────────────

/// `GET /decayed` — list decayed chunks (id, content_hash, expires_at, reason)
/// for operator review. `brain sweep --list` wraps it. Nothing is ever deleted
/// autonomously.
///
/// v1.17.1 "Govern" M2: the review list now surfaces *why* a chunk is decayed —
/// `per_chunk` (its own `expires_at` elapsed) or `kind_policy` (no `expires_at`,
/// but the kind-level retention default elapsed). The effective expiry is
/// computed at query time, the same way retrieval excludes it.
pub async fn list_decayed(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<Vec<serde_json::Value>>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;
    let now = chrono::Utc::now().timestamp();
    // Kind policy (empty when disabled → per_chunk only, exact v1.14 behavior).
    let retention_days = if crate::config::brain_retention_enabled() {
        crate::config::retention_kind_days()
    } else {
        std::collections::BTreeMap::new()
    };

    let rows =
        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, content_hash, expires_at, node_kind,
                            strftime('%s', COALESCE(created_at, '1970-01-01 00:00:00'))
                     FROM knowledge ORDER BY id",
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();
            let mut out = Vec::new();
            for (id, content_hash, expires_at, kind, created_unix) in rows {
                let effective = crate::gate::effective_expiry(
                    expires_at,
                    Some(created_unix),
                    &kind,
                    &retention_days,
                );
                if effective.is_some_and(|e| e < now) {
                    out.push(serde_json::json!({
                        "id": id,
                        "content_hash": content_hash,
                        "expires_at": expires_at,
                        "effective_expiry": effective,
                        "memory_kind": kind,
                        "reason": crate::gate::retention_reason(expires_at, effective),
                    }));
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(rows))
}

/// `POST /purge` — audited, hard, explicit-only deletion. Two bodies:
/// `{"ids": [...]}` (specific chunks) or `{"owner": "<principal>"}` (every
/// record owned by that principal). One transaction removes knowledge +
/// vec_knowledge + graph + proposals refs; a `tombstones` row + `/audit` event
/// keep the chain verifiable. No escape hatch: purged = gone from recall,
/// search, graph, AND historical `?at=` recall.
#[derive(Debug, Deserialize)]
pub struct PurgeRequest {
    #[serde(default)]
    pub ids: Vec<i64>,
    #[serde(default)]
    pub owner: Option<String>,
}

pub async fn purge(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<PurgeRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    // Purging is Admin (irreversible). Loopback/opaque = superuser (back-compat).
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    if req.ids.is_empty() && req.owner.is_none() {
        return Err(HandlerError::bad_request(
            "no_target",
            "purge requires ids or owner",
        ));
    }
    if req.ids.len() > MAX_PURGE_IDS {
        return Err(HandlerError::bad_request(
            "too_many_ids",
            format!("purge accepts at most {MAX_PURGE_IDS} ids"),
        ));
    }
    if !req.ids.is_empty() && req.owner.is_some() {
        return Err(HandlerError::bad_request(
            "ambiguous_target",
            "purge accepts ids OR owner, not both",
        ));
    }
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;

    let count = tokio::task::spawn_blocking(move || -> Result<i64, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let now = chrono::Utc::now().timestamp();

        // Resolve target ids: explicit list, or owner-anchored (M4).
        let ids: Vec<i64> = if let Some(owner) = &req.owner {
            let mut stmt = tx
                .prepare("SELECT id FROM knowledge WHERE owner = ?1")
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let mut collected = Vec::new();
            {
                let rows = stmt
                    .query_map(rusqlite::params![owner], |r| r.get::<_, i64>(0))
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                for v in rows.flatten() {
                    collected.push(v);
                }
            }
            collected
        } else {
            req.ids.clone()
        };
        if ids.is_empty() {
            return Err(HandlerError::not_found("no matching chunks to purge"));
        }

        let purged = purge_chunk_ids(&tx, &ids, now, "explicit", None)?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            "api",
            &format!("purge:{purged}"),
            crate::audit::AuditStatus::Ok,
            "purge",
        );
        Ok(purged)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(serde_json::json!({ "purged": count })))
}

/// Shared hard-delete for a list of chunk ids, run inside the caller's
/// transaction. Removes the `knowledge` row + its `vec_knowledge` embedding +
/// graph nodes/edges + supersession/derivation pointers + `proposals`
/// references, and appends a tombstone row (hash-only). Used by `/purge`
/// (reason `explicit`) and the DSAR workflow (reason `owner:<subject>`, with
/// derived descendants carrying `derived` + the purge root's origin id).
/// Returns the number of chunks actually deleted.
pub(crate) fn purge_chunk_ids(
    tx: &rusqlite::Transaction,
    ids: &[i64],
    now: i64,
    reason: &str,
    origin_id: Option<i64>,
) -> Result<i64, HandlerError> {
    let mut purged = 0i64;
    for id in ids {
        // Capture the content_hash for the tombstone before deletion.
        let content_hash: Option<String> = tx
            .query_row(
                "SELECT content_hash FROM knowledge WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        // Graph nodes/edges + supersession pointers cascade via FKs or are
        // swept explicitly; vec0 rows are deleted by knowledge_id.
        let _ = tx.execute(
            "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
            rusqlite::params![id],
        );
        let _ = tx.execute(
            "DELETE FROM relationships WHERE knowledge_id = ?1 OR from_entity_id IN (SELECT id FROM entities WHERE knowledge_id = ?1) OR to_entity_id IN (SELECT id FROM entities WHERE knowledge_id = ?1)",
            rusqlite::params![id],
        );
        let _ = tx.execute(
            "DELETE FROM evidence_links WHERE from_chunk = ?1 OR to_chunk = ?1",
            rusqlite::params![id],
        );
        let _ = tx.execute(
            "DELETE FROM proposals WHERE conflict_with = ?1",
            rusqlite::params![id],
        );
        // v1.16.1: cascade to recall_traces. The trace side table (read-event
        // replay artifact) embeds hit chunk ids in its JSON; a purged chunk
        // must not leave a trace that still "proves" it was returned. JSON1
        // is compiled into the bundled SQLite (rusqlite "bundled"), so the
        // path filter is exact, not a LIKE. Best-effort: a trace with an
        // unparseable JSON body is skipped rather than failing the purge.
        let _ = tx.execute(
            "DELETE FROM recall_traces WHERE audit_id IN (
                 SELECT rt.audit_id FROM recall_traces rt
                  WHERE json_valid(rt.trace_json)
                    AND EXISTS (
                        SELECT 1 FROM json_each(rt.trace_json, '$.hits')
                         WHERE json_extract(value, '$.id') = ?1
                    )
             )",
            rusqlite::params![id],
        );
        let n = tx
            .execute("DELETE FROM knowledge WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        if n > 0 {
            tx.execute(
                "INSERT INTO tombstones(knowledge_id, content_hash, purged_at, reason, origin_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    id,
                    content_hash.unwrap_or_else(|| "unknown".into()),
                    now,
                    reason,
                    origin_id
                ],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
            purged += 1;
        }
    }
    Ok(purged)
}

/// v1.17.3 "UMP" M2: the shared `knowledge` column list for row rendering
/// (export + the `/ump/*` record paths) — one source of truth so the record
/// engine never misses a column the export carries.
pub(crate) const KNOWLEDGE_ROW_COLS: &str =
    "id, content, node_kind, source, origin, authority, assertion_kind, confidence,
        access_scope, owner, observed_at, valid_from, valid_to,
        content_hash, title, expires_at, created_at, ump_meta, ump_id";

/// Row → the JSON shape the record engine (`emit_record`) renders from.
pub(crate) fn knowledge_row_to_json(r: &rusqlite::Row) -> rusqlite::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "id": r.get::<_, i64>(0)?,
        "content": r.get::<_, String>(1)?,
        "memory_kind": r.get::<_, String>(2)?,
        "source": r.get::<_, String>(3)?,
        "origin": r.get::<_, String>(4)?,
        "authority": r.get::<_, Option<f32>>(5)?,
        "assertion_kind": r.get::<_, String>(6)?,
        "confidence": r.get::<_, f32>(7)?,
        "access_scope": r.get::<_, String>(8)?,
        "owner": r.get::<_, Option<String>>(9)?,
        "observed_at": r.get::<_, Option<String>>(10)?,
        "valid_from": r.get::<_, Option<String>>(11)?,
        "valid_to": r.get::<_, Option<String>>(12)?,
        "content_hash": r.get::<_, Option<String>>(13)?,
        "title": r.get::<_, Option<String>>(14)?,
        "expires_at": r.get::<_, Option<i64>>(15)?,
        "created_at": r.get::<_, Option<String>>(16)?
            .map(|s| crate::consolidate::observed_secs(&Some(s)))
            .filter(|&ts| ts != 0),
        "ump_meta": r.get::<_, Option<String>>(17)?,
        "ump_id": r.get::<_, Option<String>>(18)?,
    }))
}

/// One knowledge row by id (same columns as the export) — the `/ump/*`
/// record paths resolve rows through this.
pub(crate) fn load_knowledge_row(
    conn: &rusqlite::Connection,
    id: i64,
) -> Result<Option<serde_json::Value>, rusqlite::Error> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {KNOWLEDGE_ROW_COLS} FROM knowledge WHERE id = ?1"
    ))?;
    let mut rows = stmt.query_map(rusqlite::params![id], knowledge_row_to_json)?;
    rows.next().transpose()
}

/// `GET /export` — portable, machine-readable JSON export (data portability,
/// the GDPR "give me my data"). Live `knowledge` rows + graph + proposals
/// ledger. `pii_map` values excluded by default; only included with
/// `?include_pii_map=true` AND a `pii:read` principal.
#[derive(Debug, Default, Deserialize)]
pub struct ExportQuery {
    #[serde(default)]
    pub include_pii_map: bool,
    /// v1.17.1 "Govern" M4: `?format=ump` re-renders the payload as UMP records.
    #[serde(default)]
    pub format: Option<String>,
}

pub async fn export(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    cap: OptCapability,
    Query(q): Query<ExportQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    // v1.17.3 M5: export paths require the `export` verb (§5.2).
    super::cap_gate(&cap.0, "export")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;
    // M2: resolved before the `move` closure so the export path can redact
    // records the principal doesn't own (§2.7).
    let redact_owner = principal.0.as_ref().map(|p| p.sub.clone());

    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        // v1.20.2 D3: pre-flight row count to bound memory. The full export
        // buffers every row into a Vec<Value> then serializes; on a multi-GB
        // DB that OOMs the server. We refuse (413) above MAX_EXPORT_ROWS and
        // document the per-domain `GET /domains/{name}/export` path (a single
        // domain's snapshot) + the future streaming variant. ponytail: a true
        // streaming JSON encoder is a v2.x change; this guard prevents the OOM
        // today without rewriting the response shape callers depend on.
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        if total > MAX_EXPORT_ROWS {
            return Err(HandlerError::payload_too_large(format!(
                "export exceeds {MAX_EXPORT_ROWS} rows ({total} present); use GET /domains/{{name}}/export for a single domain snapshot"
            )));
        }
        let mut knowledge = Vec::new();
        {
            let mut stmt = conn
                .prepare(&format!("SELECT {KNOWLEDGE_ROW_COLS} FROM knowledge ORDER BY id"))
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], knowledge_row_to_json)
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                knowledge.push(v);
            }
        }
        let mut proposals = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, content, novelty, conflict_with, salience, status,
                            created_at, decided_at
                     FROM proposals ORDER BY id",
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "kind": r.get::<_, String>(1)?,
                        "content": r.get::<_, String>(2)?,
                        "novelty": r.get::<_, f32>(3)?,
                        "conflict_with": r.get::<_, Option<i64>>(4)?,
                        "salience": r.get::<_, f32>(5)?,
                        "status": r.get::<_, String>(6)?,
                        "created_at": r.get::<_, i64>(7)?,
                        "decided_at": r.get::<_, Option<i64>>(8)?,
                    }))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                proposals.push(v);
            }
        }
        let mut entities = Vec::new();
        {
            let mut stmt = conn
                .prepare("SELECT id, name, entity_type FROM entities ORDER BY id")
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "entity_type": r.get::<_, Option<String>>(2)?,
                    }))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                entities.push(v);
            }
        }
        let mut edges = Vec::new();
        {
            let mut stmt = conn
                .prepare("SELECT id, from_entity_id, to_entity_id, relation_type, knowledge_id FROM relationships ORDER BY id")
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "from_entity_id": r.get::<_, i64>(1)?,
                        "to_entity_id": r.get::<_, i64>(2)?,
                        "relation_type": r.get::<_, String>(3)?,
                        "knowledge_id": r.get::<_, Option<i64>>(4)?,
                    }))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                edges.push(v);
            }
        }
        let include_pii = q.include_pii_map && crate::gate::has_pii_read(&principal.0);
        // Only resolve the pii_map when both the flag AND the principal allow it.
        let pii_map = if include_pii {
            let mut map = std::collections::BTreeMap::new();
            let mut stmt = conn
                .prepare("SELECT placeholder, value FROM pii_map ORDER BY placeholder")
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                map.insert(v.0, v.1);
            }
            serde_json::to_value(map).unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::Value::Null
        };
        // v1.18.2 "Transparency" M1: provenance summary counts per origin/source
        // kind (the Art 50 model-vs-human bridge) + additive format version.
        let mut by_origin: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
        let mut by_source: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
        for k in &knowledge {
            if let Some(o) = k["origin"].as_str() {
                *by_origin.entry(o).or_insert(0) += 1;
            }
            if let Some(s) = k["source"].as_str() {
                *by_source.entry(s).or_insert(0) += 1;
            }
        }
        Ok(serde_json::json!({
            "export_format_version": 2,
            "exported_at": chrono::Utc::now().to_rfc3339(),
            "knowledge": knowledge,
            "entities": entities,
            "relationships": edges,
            "proposals": proposals,
            "pii_map": pii_map,
            "provenance_summary": {
                "total": by_origin.values().sum::<u64>(),
                "by_origin": by_origin,
                "by_source": by_source,
            },
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    // v1.17.3 "UMP" M2: `?format=ump` re-renders the payload as signed/redactable
    // UMP records (per-chunk graph included, name-based, so a UMP peer can
    // restore it). M5 wires the operator signer here.
    if let Some(fmt) = q.format.as_deref() {
        if fmt == "ump" || fmt == "ump-md" {
            let rendered = render_ump(&body, redact_owner.as_deref(), None);
            if fmt == "ump-md" {
                // v1.17.3 M4 (§6.3): the markdown projection per record,
                // records joined by the `\n---\n` separator.
                // ponytail: a body containing a bare `---` line (setext /
                // thematic break) is a documented split ceiling.
                let content = render_ump_md(&body, redact_owner.as_deref())
                    .map_err(HandlerError::internal)?;
                let n = rendered["records"].as_array().map(|a| a.len()).unwrap_or(0);
                return Ok(Json(json!({
                    "ump": "1.0",
                    "format": "ump-md",
                    "records": n,
                    "content": content,
                })));
            }
            return Ok(Json(rendered));
        }
        return Err(HandlerError::bad_request(
            "unknown_format",
            "format must be 'ump' or 'ump-md'",
        ));
    }

    Ok(Json(body))
}

/// Pure M4 renderer (M2-hardened): `/export` body → UMP envelope. Relation
/// names are resolved through the entity map; a dangling id drops (defensive).
/// Every record goes through `emit_record` (content-addressed id + integrity +
/// §2.7 redaction for non-owner principals: a JWT subject only ever exports
/// their own rows unredacted; loopback/operator exports stay full).
fn render_ump(
    body: &serde_json::Value,
    redact_owner: Option<&str>,
    signer: Option<(&str, &ed25519_dalek::SigningKey)>,
) -> serde_json::Value {
    let entities = body["entities"].as_array().cloned().unwrap_or_default();
    let edges = body["relationships"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let name_of: std::collections::HashMap<i64, String> = entities
        .iter()
        .filter_map(|e| Some((e["id"].as_i64()?, e["name"].as_str()?.to_string())))
        .collect();
    let graph_by_chunk: std::collections::HashMap<i64, Vec<serde_json::Value>> = {
        let mut m: std::collections::HashMap<i64, Vec<serde_json::Value>> =
            std::collections::HashMap::new();
        for e in edges {
            let Some(kid) = e["knowledge_id"].as_i64() else {
                continue;
            };
            if let (Some(from), Some(to)) =
                (e["from_entity_id"].as_i64(), e["to_entity_id"].as_i64())
            {
                m.entry(kid).or_default().push(serde_json::json!({
                    "from": name_of.get(&from).cloned().unwrap_or_default(),
                    "to": name_of.get(&to).cloned().unwrap_or_default(),
                    "type": e["relation_type"].as_str().unwrap_or("relates_to"),
                }));
            }
        }
        m
    };
    let records: Vec<serde_json::Value> = body["knowledge"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|row| {
            let id = row["id"].as_i64().unwrap_or(0);
            let rels = graph_by_chunk.get(&id).cloned().unwrap_or_default();
            let meta = crate::handlers::ump::UmpMeta::parse(row["ump_meta"].as_str());
            let row_owner = row["owner"].as_str().or(meta.owner.as_deref());
            let redact = redact_owner.is_some()
                && row_owner.map(|o| Some(o) != redact_owner).unwrap_or(true);
            crate::handlers::ump::emit_record(
                row,
                "global",
                &serde_json::json!([]),
                &serde_json::json!(rels),
                &meta,
                redact,
                // `ponytail:` the export renderer is pure (no DB handle), so
                // it can't resolve `supersedes` links; the `/ump/*` ops surface
                // carries `superseded_by` on live reads.
                &[],
                signer,
            )
        })
        .collect();
    serde_json::json!({
        "ump": "1.0",
        "exported_at": body["exported_at"],
        "records": records,
    })
}

/// v1.17.3 M4: `?format=ump-md` — the §6.3 markdown projection per record,
/// joined by the `\n---\n` record separator (pure; the handler wires it).
fn render_ump_md(body: &serde_json::Value, redact_owner: Option<&str>) -> Result<String, String> {
    let rendered = render_ump(body, redact_owner, None);
    let parts = rendered["records"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(crate::handlers::ump::record_to_markdown)
        .collect::<Result<Vec<String>, String>>()?;
    Ok(parts.join(crate::handlers::ump::MD_RECORD_SEP))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v1.20.14 "Steer": the edit-audit detail is SHA-256 of before+after
    /// content (hashes only — never raw text), and it is deterministic so a
    /// replay of the same edit produces the same audit hash.
    #[test]
    fn sha256_hex_is_deterministic_hex_of_content() {
        // Known SHA-256 vectors (`echo -n ... | shasum -a 256`):
        assert_eq!(
            sha256_hex("brain"),
            "bbbf7a6412d6d3e8244ac1fda5e35a20037acee661288cb95b7b18cf469980aa"
        );
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // Determinism: same input → same output every time.
        assert_eq!(
            sha256_hex("a different body"),
            sha256_hex("a different body")
        );
    }

    #[test]
    fn principal_to_owner_maps_sub_and_none() {
        assert_eq!(principal_to_owner(&None), None);
        let p = crate::auth::Principal {
            sub: "user-42".to_string(),
            tenant: "alpha".to_string(),
            scopes: vec![],
            jti: "token-1".to_string(),
        };
        assert_eq!(principal_to_owner(&Some(p)), Some("user-42".to_string()));
    }

    /// M4: `/export` body → UMP envelope resolves relation names and attaches
    /// each chunk's graph to its own record (relations only land on the chunk
    /// they were anchored to; the raw name survives via body.structured).
    #[test]
    fn render_ump_attaches_name_based_graph_per_chunk() {
        let body = serde_json::json!({
            "exported_at": "2026-08-09T00:00:00Z",
            "knowledge": [
                {"id": 1, "content": "Dave works at Acme.", "title": "d1", "memory_kind": "fact", "created_at": 1},
                {"id": 2, "content": "Carol runs the lab.", "title": "d2", "memory_kind": "fact", "created_at": 2},
            ],
            "entities": [
                {"id": 10, "name": "Dave", "entity_type": "person"},
                {"id": 11, "name": "Acme", "entity_type": "org"},
            ],
            "relationships": [
                {"id": 100, "from_entity_id": 10, "to_entity_id": 11, "relation_type": "works_at", "knowledge_id": 1},
                {"id": 101, "from_entity_id": 10, "to_entity_id": 11, "relation_type": "works_at", "knowledge_id": 2},
            ],
        });
        let out = render_ump(&body, None, None);
        assert_eq!(out["ump"], "1.0");
        let recs = out["records"].as_array().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(
            recs[0]["body"]["structured"]["relations"][0],
            serde_json::json!({"from": "Dave", "to": "Acme", "type": "works_at"})
        );
        assert_eq!(
            recs[1]["body"]["structured"]["relations"][0],
            serde_json::json!({"from": "Dave", "to": "Acme", "type": "works_at"})
        );
        assert_eq!(recs[0]["id"], "urn:ump:brain:global:1");
        assert_eq!(recs[1]["id"], "urn:ump:brain:global:2");
    }

    /// M2: the export renderer now emits integrity-protected records (§2.7) —
    /// a JWT subject exporting sees their own rows unredacted and other rows
    /// redacted (still shape- and integrity-valid).
    #[test]
    fn render_ump_md_round_trips_through_record_from_markdown() {
        let body = serde_json::json!({
            "exported_at": "2026-08-09T00:00:00Z",
            "knowledge": [
                {"id": 1, "content": "Dave works at Acme.", "title": "d1", "memory_kind": "fact", "created_at": 1},
                {"id": 2, "content": "Carol runs the lab.", "title": "d2", "memory_kind": "fact", "created_at": 2},
            ],
            "entities": [],
            "relationships": [],
        });
        let md = render_ump_md(&body, None).expect("md render");
        assert!(md.contains("ump: \"1.0\""), "frontmatter present: {md}");
        let chunks: Vec<&str> = md.split(crate::handlers::ump::MD_RECORD_SEP).collect();
        assert_eq!(chunks.len(), 2, "one projection per record");
        let rec0 = crate::handlers::ump::record_from_markdown(chunks[0]).expect("parse rec 0");
        assert_eq!(rec0["body"]["text"], "Dave works at Acme.");
        assert_eq!(rec0["kind"], "semantic");
        assert_eq!(rec0["body"]["structured"]["title"], "d1");
        let rec1 = crate::handlers::ump::record_from_markdown(chunks[1]).expect("parse rec 1");
        assert_eq!(rec1["body"]["text"], "Carol runs the lab.");
    }

    /// M2: the export renderer now emits integrity-protected records (§2.7) —
    /// a JWT subject exporting sees their own rows unredacted and other rows
    /// redacted (still shape- and integrity-valid).
    #[test]
    fn render_ump_redacts_records_not_owned_by_the_principal() {
        let body = serde_json::json!({
            "exported_at": "2026-08-09T00:00:00Z",
            "knowledge": [
                {"id": 1, "content": "Mine.", "title": "d1", "memory_kind": "fact", "owner": "user-1", "created_at": 1},
                {"id": 2, "content": "Theirs.", "title": "d2", "memory_kind": "fact", "owner": "user-2", "created_at": 2},
                {"id": 3, "content": "No owner.", "title": "d3", "memory_kind": "fact", "created_at": 3},
            ],
            "entities": [],
            "relationships": [],
        });
        let out = render_ump(&body, Some("user-1"), None);
        let recs = out["records"].as_array().unwrap();
        assert_eq!(recs[0]["body"]["text"], "Mine.");
        assert_eq!(recs[1]["body"]["text"], "[redacted]");
        assert_eq!(recs[2]["body"]["text"], "[redacted]");
        for r in recs {
            assert!(
                r["integrity"]["content_hash"].is_string(),
                "integrity present"
            );
            assert!(
                crate::handlers::ump::verify_record(r, None),
                "redacted record still verifies"
            );
        }
    }

    /// Regression: v1.17.1 M4 added `created_at` to the export SELECT, but the
    /// column is TEXT (`CURRENT_TIMESTAMP` default) while the mapper read it
    /// as `Option<i64>` — every row errored and `flatten()` silently dropped
    /// them all, so `/export` (and the UMP re-render) returned an empty
    /// `knowledge` list on any real DB. The mapper now parses the DB
    /// timestamp; this test pins the real migration + INSERT + export mapping.
    #[test]
    fn export_mapping_survives_real_timestamp_rows() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().expect("db");
        brain_server::migration::run_migration(&mut conn, 1).expect("migration");
        conn.execute(
            "INSERT INTO knowledge (title, content, source, content_hash) \
             VALUES ('d1', 'Dave works at Acme.', 'structured', 'abc123')",
            [],
        )
        .expect("insert");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {KNOWLEDGE_ROW_COLS} FROM knowledge ORDER BY id"
            ))
            .expect("prepare");
        let rows: Vec<serde_json::Value> = stmt
            .query_map([], knowledge_row_to_json)
            .expect("query")
            .flatten()
            .collect();
        assert_eq!(rows.len(), 1, "the row must survive the mapping");
        assert_eq!(rows[0]["content"], "Dave works at Acme.");
        assert!(
            rows[0]["created_at"].is_i64(),
            "created_at is a unix epoch: {}",
            rows[0]["created_at"]
        );
    }

    /// v1.18.2 M1: export JSON carries per-row `source` + `origin` + the
    /// provenance_summary envelope + export_format_version 2, while all v1
    /// field names survive (regression guard for downstream importers).
    #[test]
    fn export_contains_source_origin_and_provenance_summary() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().expect("db");
        brain_server::migration::run_migration(&mut conn, 1).expect("migration");
        // One chunk per ingest kind so the summary counts are meaningful.
        // origin mirrors what the write-time handlers set (manual→human,
        // memory→model, markdown/structured→imported default).
        conn.execute(
            "INSERT INTO knowledge (title, content, source, content_hash, origin) \
             VALUES ('m', 'manual row', 'manual', 'h-m', 'human')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO knowledge (title, content, source, content_hash, origin) \
             VALUES ('m2', 'model row', 'memory', 'h-m2', 'model')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO knowledge (title, content, source, content_hash) \
             VALUES ('s', 'structured row', 'structured', 'h-s')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO knowledge (title, content, source, content_hash) \
             VALUES ('md', 'markdown row', 'markdown', 'h-md')",
            [],
        )
        .unwrap();

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {KNOWLEDGE_ROW_COLS} FROM knowledge ORDER BY id"
            ))
            .unwrap();
        let knowledge: Vec<serde_json::Value> = stmt
            .query_map([], knowledge_row_to_json)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(knowledge.len(), 4);

        let by_origin: std::collections::HashMap<&str, usize> = knowledge
            .iter()
            .map(|k| (k["origin"].as_str().unwrap(), 1))
            .fold(std::collections::HashMap::new(), |mut m, (o, n)| {
                *m.entry(o).or_insert(0) += n;
                m
            });
        assert_eq!(by_origin.get("human"), Some(&1));
        assert_eq!(by_origin.get("model"), Some(&1));
        assert_eq!(by_origin.get("imported"), Some(&2));

        // Manual → human; memory → model; markdown/structured → imported.
        assert_eq!(knowledge[0]["source"], "manual");
        assert_eq!(knowledge[0]["origin"], "human");
        assert_eq!(knowledge[1]["source"], "memory");
        assert_eq!(knowledge[1]["origin"], "model");
        assert_eq!(knowledge[2]["source"], "structured");
        assert_eq!(knowledge[2]["origin"], "imported");
        assert_eq!(knowledge[3]["source"], "markdown");
        assert_eq!(knowledge[3]["origin"], "imported");

        // Every v1 field name still present with the same name.
        for field in [
            "id",
            "content",
            "memory_kind",
            "authority",
            "assertion_kind",
            "confidence",
            "access_scope",
            "owner",
            "observed_at",
            "valid_from",
            "valid_to",
            "content_hash",
        ] {
            assert!(
                knowledge[0].get(field).is_some(),
                "v1 field {field} must survive"
            );
        }
    }

    /// v1.18.2 M2: the migration backfills `origin` by source kind.
    #[test]
    fn migration_backfills_origin_by_source() {
        crate::register_sqlite_vec();
        // Build a pre-origin DB by running the migration, then dropping origin,
        // seeding rows of each kind, and re-running the migration to backfill.
        let mut conn = rusqlite::Connection::open_in_memory().expect("db");
        brain_server::migration::run_migration(&mut conn, 1).expect("migration");
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_knowledge_origin;
             ALTER TABLE knowledge DROP COLUMN origin;
             INSERT INTO knowledge (content, source, content_hash) VALUES
                ('a', 'manual', 'h1'),
                ('b', 'memory', 'h2'),
                ('c', 'markdown', 'h3'),
                ('d', 'structured', 'h4'),
                ('e', 'weird', 'h5');",
        )
        .unwrap();
        brain_server::migration::run_migration(&mut conn, 1).expect("re-migration");
        let origin: Vec<String> = conn
            .prepare("SELECT origin FROM knowledge ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|v| v.unwrap())
            .collect();
        assert_eq!(
            origin,
            vec!["human", "model", "imported", "imported", "imported"]
        );
    }
}
