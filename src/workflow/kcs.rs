//! The KCS Solve-loop substrate (the Evolve release).
//!
//! Three deterministic writes, all evidence-derived, all audited:
//! 1. **SIR records** — every reuse search during a run lands a
//!    `case_articles` row (`searched_found` per cited hit,
//!    `searched_not_found` on the zero-hit abstention). The Reuse practice.
//! 2. **Improve flags** — a completed run that contradicted its cited
//!    article emits a `kcs_flag` finding per cited article. Content-health
//!    input only; edits stay HITL.
//! 3. **Capture at close** — on `crm/case/closed`, exactly-once via the
//!    outbox idempotency key, the deterministic capture generator emits ONE
//!    structured proposal (`kcs_new_article` | `kcs_update_article` |
//!    `kcs_link_only`). A closed case with zero linkage rows is FLAGGED
//!    (`kcs_unlinked_case`) — visible on the scoreboard feed, never a veto.

use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;

use brain_engine_sdk::pure::kcs::{CaptureAction, article_body, capture_decision, preamble};

/// Outbox topic for the capture marker event (exactly-once gate + lineage).
pub(crate) const TOPIC_KCS_CAPTURE: &str = "kcs/capture";

/// The proposal-kind family emitted by capture.
pub const KIND_NEW: &str = "kcs_new_article";
pub const KIND_UPDATE: &str = "kcs_update_article";
pub const KIND_LINK_ONLY: &str = "kcs_link_only";

/// The Beacon publish proposal (`{knowledge_id, public_slug, action}` JSON in
/// `content`). Approval flips the article to `published` / back to `approved`
/// on `action=retract`. Publishing is an EXTERNAL act, so it needs its own
/// verb: approval requires the `approve` capability AND a distinct `publish`.
pub const KIND_PUBLISH: &str = "kcs_publish";

/// The human translation proposal (`{knowledge_id, locale, title, body_md}`
/// JSON in `content`). Translation is a HUMAN act — the tool governs, it
/// never machine-translates. Approval promotes the `kcs_translations` row to
/// `approved`, pinned to `based_revision` (the source revision it
/// translated) so staleness is first-class.
pub const KIND_TRANSLATE: &str = "kcs_translate";

pub(crate) const MAX_TRANSLATION_LEN: usize = 64_000;
pub(crate) const MAX_LOCALE_LEN: usize = 12;

/// The freshness-review horizon stamped at approve/publish. Single definition
/// shared by the lifecycle routes and the publish gate.
pub const KCS_FRESHNESS_SECS: i64 = 90 * 24 * 3600;

/// The `case_ref` bound to a run by the Bridges register, if any.
pub(crate) fn case_ref_for_run(conn: &Connection, run_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT case_ref FROM crm_cases WHERE run_id = ?1",
        params![run_id],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// Record `searched_found` SIR rows for the cited hits of one run's reuse
/// search. Idempotent per (case_ref, knowledge_id) via the partial unique
/// index; returns rows newly written.
pub(crate) fn record_sir_found(conn: &Connection, run_id: i64, hit_ids: &[i64], now: i64) -> usize {
    let Some(case_ref) = case_ref_for_run(conn, run_id) else {
        return 0;
    };
    let mut written = 0;
    for id in hit_ids {
        written += conn
            .execute(
                "INSERT OR IGNORE INTO case_articles(case_ref, knowledge_id, sir, action, ts)
                 VALUES (?1, ?2, 'searched_found', 'linked', ?3)",
                params![case_ref, id, now],
            )
            .unwrap_or(0);
    }
    if written > 0 {
        crate::workflow::audit_write(
            conn,
            run_id,
            &format!("kcs:{case_ref}"),
            crate::audit::AuditStatus::Ok,
            &format!("sir:searched_found x{written}"),
        );
    }
    written
}

/// Record the `searched_not_found` SIR row (the documented KCS signal for a
/// zero-hit reuse search). No article to point at — `knowledge_id` stays NULL.
pub(crate) fn record_sir_not_found(conn: &mut Connection, run_id: i64, now: i64) -> usize {
    let Some(case_ref) = case_ref_for_run(conn, run_id) else {
        return 0;
    };
    let n = match conn.execute(
        "INSERT INTO case_articles(case_ref, knowledge_id, sir, action, ts)
         VALUES (?1, NULL, 'searched_not_found', 'linked', ?2)",
        params![case_ref, now],
    ) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("sir not-found insert failed for run {run_id}: {e}");
            0
        }
    };
    if n > 0 {
        crate::workflow::audit_write(
            conn,
            run_id,
            &format!("kcs:{case_ref}"),
            crate::audit::AuditStatus::Ok,
            "sir:searched_not_found",
        );
    }
    n
}

/// The zero-hit reuse probe behind the run-suggestions surface: a bounded
/// LIKE search over the run's domain with the quarantine + decay posture
/// (flagged rows never surface through a side door; expired rows stay
/// retired) and a wildcard-injection fence (`%`/`_`/`\\` escaped, query
/// clamped). Fail-open by contract: any storage error reads as "no reuse
/// candidates", never a failed run read. Stored forms — the read seam
/// (snippet sanitize + clamp) stays handler-side.
pub(crate) fn reuse_candidates(
    conn: &Connection,
    domain: &str,
    q: &str,
    now: i64,
) -> Vec<(i64, Option<String>, String)> {
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT id,title,content FROM knowledge \
         WHERE domain=?1 AND flagged=0 \
           AND (expires_at IS NULL OR expires_at >= ?3) \
           AND content LIKE ?2 ESCAPE '\\' LIMIT 5",
    ) else {
        return out;
    };
    let q_take: String = q.chars().take(50).collect();
    let pat = format!(
        "%{}%",
        q_take
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let rows = match stmt.query_map(params![domain, pat, now], |r| {
        let id: i64 = r.get(0)?;
        let title: Option<String> = r.get(1)?;
        let content: String = r.get(2)?;
        Ok((id, title, content))
    }) {
        Ok(r) => r,
        Err(_) => return out,
    };
    for row in rows.flatten() {
        out.push(row);
    }
    out
}

/// The `abstention` finding row for a zero-hit reuse search. Best-effort at
/// the caller: the loud warn stays with the caller, who owns the context.
pub(crate) fn record_abstention(
    conn: &Connection,
    run_id: i64,
    now: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO findings(run_id,claim,evidence,source,confidence,ts) VALUES (?1,'abstention','no hits','copilot',0,?2)",
        params![run_id, now],
    )
}

/// The Improve practice: when a run COMPLETED but contradicted what it used
/// (diverged steps / skipped verification), emit one `kcs_flag` finding per
/// cited article. Returns flagged article ids. Never an edit — content-health
/// input for the stale worklist.
pub(crate) fn flag_contradicted_articles(
    conn: &mut Connection,
    run_id: i64,
    state_json: &serde_json::Value,
    now: i64,
) -> Vec<i64> {
    let contradictions = state_json
        .get("contradictions")
        .and_then(|c| c.as_u64())
        .unwrap_or(0)
        > 0;
    let skipped = state_json
        .get("steps")
        .and_then(|s| s.as_array())
        .is_some_and(|steps| {
            steps.iter().any(|s| {
                s.get("skipped_verify")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false)
            })
        });
    if !contradictions && !skipped {
        return vec![];
    }
    let Some(case_ref) = case_ref_for_run(conn, run_id) else {
        return vec![];
    };
    let cited: Vec<i64> = {
        let Ok(mut stmt) = conn.prepare(
            "SELECT DISTINCT knowledge_id FROM case_articles
             WHERE case_ref = ?1 AND sir = 'searched_found' AND knowledge_id IS NOT NULL",
        ) else {
            return vec![];
        };
        stmt.query_map(params![case_ref], |r| r.get(0))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default()
    };
    if cited.is_empty() {
        return vec![];
    }
    // Flags and their audit rows commit atomically.
    let Ok(tx) = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) else {
        tracing::warn!("improve flags: no write tx available for run {run_id}");
        return vec![];
    };
    for id in &cited {
        if let Err(e) = tx.execute(
            "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts)
             VALUES (?1, 'kcs_flag', ?2, 'kcs', 0, ?3)",
            params![run_id, format!("article:{id}"), now],
        ) {
            tracing::warn!("kcs_flag insert failed for run {run_id}: {e}");
        }
        crate::workflow::audit_write(
            &tx,
            run_id,
            &format!("article:{id}"),
            crate::audit::AuditStatus::Ok,
            &format!("kcs_flag case:{case_ref}"),
        );
    }
    if tx.commit().is_err() {
        tracing::warn!("kcs_flag commit failed for run {run_id}");
        return vec![];
    }
    cited
}

struct CaptureInputs {
    state: serde_json::Value,
    findings_claims: Vec<String>,
}

fn load_capture_inputs(conn: &Connection, run_id: i64) -> Option<CaptureInputs> {
    let (js, status): (String, String) = conn
        .query_row(
            "SELECT state_json, status FROM workflow_runs WHERE id = ?1",
            params![run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten()?;
    if status != "completed" && status != "active" {
        return None;
    }
    let state: serde_json::Value = serde_json::from_str(&js).ok()?;
    let claims: Vec<String> = conn
        .prepare("SELECT claim FROM findings WHERE run_id = ?1")
        .ok()?
        .query_map(params![run_id], |r| r.get(0))
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default();
    Some(CaptureInputs {
        state,
        findings_claims: claims,
    })
}

/// Deterministic inputs → proposal body. Pure over recorded evidence.
fn build_proposal_content(
    action: &CaptureAction,
    case_ref: &str,
    cited: &[i64],
    inputs: &CaptureInputs,
) -> String {
    let issue = inputs
        .state
        .get("is_seed")
        .and_then(|v| v.as_str())
        .or_else(|| inputs.state.get("q").and_then(|v| v.as_str()))
        .unwrap_or("unrecorded symptom");
    let environment = inputs
        .state
        .get("is_not_seed")
        .and_then(|v| v.as_str())
        .unwrap_or("not recorded");
    let cause: Vec<String> = inputs.findings_claims.clone();
    let resolution: Vec<String> = inputs
        .state
        .get("steps")
        .and_then(|s| s.as_array())
        .map(|steps| {
            steps
                .iter()
                .filter_map(|s| s.get("actual").and_then(|a| a.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut evidence: Vec<String> = cited
        .iter()
        .map(|id| format!("sir=searched_found article={id}"))
        .collect();
    evidence.push(format!("case={case_ref}"));
    match action {
        CaptureAction::NewArticle => {
            format!(
                "{}{body}",
                preamble(case_ref, None),
                body = article_body(issue, environment, &cause, &resolution, &evidence)
            )
        }
        CaptureAction::UpdateArticle => {
            let target = cited.first().copied();
            format!(
                "{}{body}",
                preamble(case_ref, target),
                body = article_body(issue, environment, &cause, &resolution, &evidence)
            )
        }
        CaptureAction::LinkOnly => {
            let target = cited.first().copied();
            format!(
                "{}{body}",
                preamble(case_ref, target),
                body = article_body(
                    issue,
                    environment,
                    &[],
                    &[],
                    &[
                        "link_only=true".to_string(),
                        format!("existing_articles={}", cited.len()),
                    ]
                    .into_iter()
                    .chain(evidence)
                    .collect::<Vec<String>>(),
                )
            )
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CaptureOutcome {
    /// One proposal created (`kind`, `proposal_id`).
    Proposed(&'static str, i64),
    /// Replay or missing prerequisites — nothing written.
    Skipped,
}

/// The close-out: exactly-once capture for a closed-solved case. Emits ONE
/// HITL proposal and enforces the linkage invariant (flag, never block).
/// Caller owns the transaction/commit discipline; every write here is
/// same-tx with its audit row.
pub(crate) fn capture_on_case_close(
    conn: &mut Connection,
    run_id: i64,
    now: i64,
) -> Result<CaptureOutcome, rusqlite::Error> {
    // Exactly once per run: the outbox marker is the idempotency gate.
    let Some(case_ref) = case_ref_for_run(conn, run_id) else {
        return Ok(CaptureOutcome::Skipped);
    };
    let status: String = conn
        .query_row(
            "SELECT status FROM crm_cases WHERE case_ref = ?1",
            params![case_ref],
            |r| r.get(0),
        )
        .unwrap_or_default();
    if status != "closed_solved" {
        return Ok(CaptureOutcome::Skipped);
    }
    // Complaint clusters are the highest-value KCS input — a
    // complaint run captures as `complaint_rca` with the cluster-boosted
    // salience (pure::complaint::capture_salience; complaint clusters
    // outrank incident repeaters deterministically).
    let run_kind: String = conn
        .query_row(
            "SELECT kind FROM workflow_runs WHERE id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let is_complaint = run_kind == "complaint";
    let cluster_count = if is_complaint {
        conn.query_row(
            "SELECT COUNT(*) FROM workflow_runs
              WHERE kind = 'complaint' AND created_at BETWEEN ?1 - 30*86400 AND ?1",
            params![now],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    } else {
        0
    };
    let salience = brain_engine_sdk::pure::complaint::capture_salience(is_complaint, cluster_count);
    let key = format!("kcs-capture-{case_ref}");
    let payload = serde_json::json!({"case_ref": case_ref, "run_id": run_id}).to_string();
    // The whole close-out — marker, proposal, linkage-invariant finding,
    // and every audit row — commits atomically or not at all.
    // Read-only preconditions BEFORE the tx opens (no borrow overlap).
    let inputs = load_capture_inputs(conn, run_id);
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let (first, _marker) =
        crate::workflow::outbox::enqueue(&tx, run_id, TOPIC_KCS_CAPTURE, &payload, &key, now)?;
    if !first {
        return Ok(CaptureOutcome::Skipped);
    }
    let Some(inputs) = inputs else {
        return Ok(CaptureOutcome::Skipped);
    };
    let cited: Vec<i64> = {
        let mut stmt =
            tx.prepare("SELECT DISTINCT knowledge_id FROM case_articles WHERE case_ref = ?1 AND sir = 'searched_found' AND knowledge_id IS NOT NULL")?;
        stmt.query_map(params![case_ref], |r| r.get(0))?
            .filter_map(Result::ok)
            .collect::<Vec<i64>>()
    };
    let diverged = inputs
        .state
        .get("steps")
        .and_then(|s| s.as_array())
        .map(|steps| {
            steps
                .iter()
                .filter(|s| s.get("expected") != s.get("actual"))
                .count()
        })
        .unwrap_or(0);
    let action = capture_decision(!cited.is_empty(), None, diverged);

    let kind = if is_complaint {
        crate::workflow::complaint::KIND_RCA
    } else {
        match action {
            CaptureAction::NewArticle => KIND_NEW,
            CaptureAction::UpdateArticle => KIND_UPDATE,
            CaptureAction::LinkOnly => KIND_LINK_ONLY,
        }
    };
    let content = build_proposal_content(&action, &case_ref, &cited, &inputs);
    tx.execute(
        "INSERT INTO proposals(kind, content, source, authority, observed_at,
                              novelty, conflict_with, salience, created_at, source_prompt, owner)
         VALUES (?1, ?2, 'agent', NULL, NULL, 1.0, NULL, ?3, ?4, NULL, NULL)",
        params![kind, content, salience, now],
    )?;
    let proposal_id = tx.last_insert_rowid();

    // Linkage invariant: flag a closed case with zero linked articles.
    // Visible, never a veto.
    let linked: i64 = tx.query_row(
        "SELECT COUNT(*) FROM case_articles WHERE case_ref = ?1 AND knowledge_id IS NOT NULL",
        params![case_ref],
        |r| r.get(0),
    )?;
    if linked == 0 {
        tx.execute(
            "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts)
             VALUES (?1, 'kcs_unlinked_case', ?2, 'kcs', 0, ?3)",
            params![run_id, case_ref, now],
        )?;
    }

    crate::workflow::audit_write(
        &tx,
        run_id,
        &format!("proposal:{proposal_id}"),
        crate::audit::AuditStatus::Ok,
        &format!(
            "kcs/capture {kind} case:{case_ref} unlinked:{}",
            linked == 0
        ),
    );
    tx.commit()?;
    Ok(CaptureOutcome::Proposed(kind, proposal_id))
}

/// Parse the machine preamble the approve path uses to wire the linkage.
/// Returns `(case_ref, article_id)`.
pub(crate) fn parse_preamble(content: &str) -> Option<(String, Option<i64>)> {
    let mut case_ref = None;
    let mut article = None;
    for line in content.lines().take(8) {
        if let Some(v) = line.strip_prefix("kcs: case=") {
            case_ref = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("kcs: article=") {
            article = v.trim().parse().ok();
        } else if line.starts_with('#') {
            break;
        }
    }
    case_ref.map(|c| (c, article))
}

/// The distinct cited-article ids across SIR rows (helper for tests/routes).
#[allow(dead_code)]
pub(crate) fn cited_articles(conn: &Connection, case_ref: &str) -> HashSet<i64> {
    let mut stmt = match conn.prepare(
        "SELECT DISTINCT knowledge_id FROM case_articles
         WHERE case_ref = ?1 AND sir = 'searched_found' AND knowledge_id IS NOT NULL",
    ) {
        Ok(s) => s,
        Err(_) => return Default::default(),
    };
    stmt.query_map(params![case_ref], |r| r.get(0))
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// The Evolve-loop performance measures (scoreboard + weekly report).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KcsMeasures {
    /// linked closed cases ÷ closed cases × SCALE.
    pub linkage_rate_units: i32,
    /// searched_found ÷ all SIR records × SCALE (the reuse rate).
    pub searched_found_rate_units: i32,
    /// median seconds since creation across living KCS articles.
    pub article_freshness_median_age_secs: i64,
}

use brain_engine_sdk::pure::qa_score::SCALE;

/// Deterministic measures over the linkage + SIR + article tables.
/// Zero denominators read as 0 (no evidence is not good evidence).
pub(crate) fn kcs_measures(conn: &Connection, now: i64) -> rusqlite::Result<KcsMeasures> {
    let closed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM crm_cases WHERE status = 'closed_solved'",
        [],
        |r| r.get(0),
    )?;
    let linked_closed: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT case_ref) FROM crm_cases c
         WHERE c.status = 'closed_solved'
           AND EXISTS (SELECT 1 FROM case_articles a
                        WHERE a.case_ref = c.case_ref AND a.knowledge_id IS NOT NULL)",
        [],
        |r| r.get(0),
    )?;
    let linkage_rate_units = if closed == 0 {
        0
    } else {
        (linked_closed * SCALE as i64 / closed) as i32
    };
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM case_articles WHERE sir = 'searched_found'",
        [],
        |r| r.get(0),
    )?;
    let not_found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM case_articles WHERE sir = 'searched_not_found'",
        [],
        |r| r.get(0),
    )?;
    let searched_found_rate_units = if found + not_found == 0 {
        0
    } else {
        (found * SCALE as i64 / (found + not_found)) as i32
    };
    let mut ages: Vec<i64> = {
        let mut stmt =
            conn.prepare("SELECT created_at FROM knowledge WHERE kcs_state != 'none' ORDER BY id")?;
        stmt.query_map([], |r| r.get::<_, i64>(0))?
            .filter_map(Result::ok)
            .collect()
    };
    let median = if ages.is_empty() {
        0
    } else {
        ages.sort_unstable();
        let mid = ages.len() / 2;
        now.saturating_sub(ages[mid])
    };
    Ok(KcsMeasures {
        linkage_rate_units,
        searched_found_rate_units,
        article_freshness_median_age_secs: median,
    })
}

/// The compact summary string the weekly calibration report carries.
pub(crate) fn kcs_summary(conn: &Connection, now: i64) -> rusqlite::Result<String> {
    let m = kcs_measures(conn, now)?;
    Ok(format!(
        "kcs_linkage_rate:{} reuse_rate:{} freshness_median_age_secs:{}",
        m.linkage_rate_units, m.searched_found_rate_units, m.article_freshness_median_age_secs
    ))
}

/// The Beacon feedback flywheel measures (aggregate counters only — each
/// `kb_feedback` finding is one anonymous helpful/not-helpful vote).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KbFeedbackMeasures {
    /// helpful ÷ total feedback × SCALE (indicative self-service deflection,
    /// NOT a savings claim — see docs/kb-deflection.md).
    pub self_service_deflection_units: i32,
    pub total_feedback: i64,
}

pub(crate) fn kb_feedback_measures(conn: &Connection) -> rusqlite::Result<KbFeedbackMeasures> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM findings WHERE claim = 'kb_feedback'",
        [],
        |r| r.get(0),
    )?;
    let helpful: i64 = conn.query_row(
        "SELECT COUNT(*) FROM findings WHERE claim = 'kb_feedback'
          AND source = 'kb-feedback:helpful'",
        [],
        |r| r.get(0),
    )?;
    Ok(KbFeedbackMeasures {
        self_service_deflection_units: if total == 0 {
            0
        } else {
            (helpful * SCALE as i64 / total) as i32
        },
        total_feedback: total,
    })
}

/// Hot topics: published slugs whose feedback volume keeps repeating — the
/// "this symptom keeps coming back" demand signal. Deterministic order
/// (count DESC, slug ASC), bounded.
pub(crate) fn kb_hot_topics(
    conn: &Connection,
    min_count: i64,
    limit: usize,
) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT f.evidence, COUNT(*) AS n FROM findings f
          WHERE f.claim = 'kb_feedback'
            AND EXISTS (SELECT 1 FROM knowledge k
                         WHERE k.public_slug = f.evidence AND k.kcs_state = 'published')
          GROUP BY f.evidence
         HAVING n >= ?1
          ORDER BY n DESC, f.evidence ASC
          LIMIT ?2",
    )?;
    stmt.query_map(params![min_count, limit as i64], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?
    .filter_map(Result::ok)
    .collect::<Vec<_>>()
    .pipe(Ok)
}

trait Pipe: Sized {
    fn pipe<F, T>(self, f: F) -> T
    where
        F: FnOnce(Self) -> T,
    {
        f(self)
    }
}
impl<T> Pipe for T {}

// ── The governed human translation surface.

/// A human-authored translation, proposed as a `kcs_translate` HITL
/// proposal. Nothing here writes `kcs_translations` directly: only an
/// APPROVED proposal does (the gate branch is the sole writer).
pub(crate) struct TranslationDraft<'a> {
    pub knowledge_id: i64,
    pub locale: &'a str,
    pub title: &'a str,
    pub body_md: &'a str,
    pub translator: &'a str,
}

fn valid_locale(l: &str) -> bool {
    !l.is_empty()
        && l.len() <= MAX_LOCALE_LEN
        && l.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// File a pending translation proposal. Bounds-checked; the source article
/// must exist. Returns the proposal id.
pub(crate) fn propose_translation(
    conn: &Connection,
    draft: &TranslationDraft<'_>,
    now: i64,
) -> rusqlite::Result<i64> {
    if !valid_locale(draft.locale) {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "locale_invalid '{}'",
            draft.locale
        )));
    }
    if draft.title.is_empty() || draft.body_md.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "translation_empty".into(),
        ));
    }
    if draft.title.len() + draft.body_md.len() > MAX_TRANSLATION_LEN {
        return Err(rusqlite::Error::InvalidParameterName(
            "translation_unbounded".into(),
        ));
    }
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM knowledge WHERE id = ?1",
            params![draft.knowledge_id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(rusqlite::Error::InvalidParameterName(
            "article_not_found".into(),
        ));
    }
    let content = serde_json::json!({
        "knowledge_id": draft.knowledge_id,
        "locale": draft.locale,
        "title": draft.title,
        "body_md": draft.body_md,
        "translator": draft.translator,
    });
    conn.execute(
        "INSERT INTO proposals(kind, content, source, authority, observed_at,
                              novelty, conflict_with, salience, created_at, source_prompt, owner)
         VALUES (?1, ?2, 'human', NULL, NULL, 1.0, NULL, 1.0, ?3, NULL, ?4)",
        params![KIND_TRANSLATE, content.to_string(), now, draft.translator],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The gate's sole promotion path for `kcs_translate`: parse the approved
/// proposal's payload and upsert the per-locale row as `approved`, pinned to
/// the source revision AT approval time (`based_revision`). Returns the
/// translation row id.
pub(crate) fn apply_translation_approval(
    tx: &Connection,
    content_json: &str,
    now: i64,
) -> rusqlite::Result<i64> {
    let v: serde_json::Value = serde_json::from_str(content_json)
        .map_err(|_| rusqlite::Error::InvalidParameterName("kcs_translate_content".into()))?;
    let knowledge_id = v
        .get("knowledge_id")
        .and_then(|x| x.as_i64())
        .ok_or_else(|| rusqlite::Error::InvalidParameterName("knowledge_id_missing".into()))?;
    let locale = v
        .get("locale")
        .and_then(|x| x.as_str())
        .filter(|l| valid_locale(l))
        .ok_or_else(|| rusqlite::Error::InvalidParameterName("locale_missing".into()))?;
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or_default();
    let body_md = v
        .get("body_md")
        .and_then(|x| x.as_str())
        .unwrap_or_default();
    if title.is_empty() || body_md.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "translation_empty".into(),
        ));
    }
    let translator = v.get("translator").and_then(|x| x.as_str()).unwrap_or("");
    // Pin the revision the translation is based on — the CURRENT source hash
    // at approval time. If the source advances past this later, the
    // worklist flags the translation stale.
    let based_revision: String = tx.query_row(
        "SELECT COALESCE(content_hash, '') FROM knowledge WHERE id = ?1",
        params![knowledge_id],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO kcs_translations(knowledge_id, locale, title, body_md,
                                      based_revision, state, translator, approved_at,
                                      created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'approved', ?6, ?7, ?7, ?7)
         ON CONFLICT(knowledge_id, locale) DO UPDATE SET
             title = excluded.title,
             body_md = excluded.body_md,
             based_revision = excluded.based_revision,
             state = 'approved',
             translator = excluded.translator,
             approved_at = excluded.approved_at,
             updated_at = excluded.updated_at",
        params![
            knowledge_id,
            locale,
            title,
            body_md,
            based_revision,
            translator,
            now
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Approved translations that went STALE: the source article's current
/// revision advanced past the pinned `based_revision`. These land on Evolve's
/// content-health worklist (the same freshness discipline, no second
/// mechanism). Deterministic order (by translation id).
pub(crate) fn stale_translations(conn: &Connection) -> rusqlite::Result<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.knowledge_id, t.locale, t.title, k.domain
         FROM kcs_translations t JOIN knowledge k ON k.id = t.knowledge_id
         WHERE t.state = 'approved' AND t.based_revision != COALESCE(k.content_hash, '')
         ORDER BY t.id LIMIT 200",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, knowledge_id, locale, title, domain)| {
            serde_json::json!({
                "translation_id": id,
                "article_id": knowledge_id,
                "locale": locale,
                "title": crate::gate::sanitize_read(&title, false, &None),
                "domain": domain,
                "kind": "translation_stale",
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::register_sqlite_vec;
    use brain_server::migration::run_migration;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("open");
        run_migration(&mut conn, 1).expect("migration");
        conn
    }

    /// Seed a run bound to a closed-solved CRM case; returns (run_id, case_ref).
    fn seed_closed_case(conn: &Connection, state_json: &str) -> (i64, String) {
        seed_closed_case_kind(conn, "interview", state_json)
    }

    fn seed_closed_case_kind(conn: &Connection, kind: &str, state_json: &str) -> (i64, String) {
        seed_closed_case_kind_at(conn, kind, state_json, 1)
    }

    fn seed_closed_case_kind_at(
        conn: &Connection,
        kind: &str,
        state_json: &str,
        created_at: i64,
    ) -> (i64, String) {
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('global', ?1, ?2, 0, 'completed', ?3, ?3)",
            params![kind, state_json, created_at],
        )
        .unwrap();
        let run_id = conn.last_insert_rowid();
        let case_ref = format!("crm:zendesk:acme:{run_id}");
        conn.execute(
            "INSERT INTO crm_cases(case_ref, source, org_id, case_id, run_id, status, updated_rev, synced_at)
             VALUES (?1, 'zendesk', 'acme', ?2, ?3, 'closed_solved', 'r1', CURRENT_TIMESTAMP)",
            params![case_ref, run_id.to_string(), run_id],
        )
        .unwrap();
        (run_id, case_ref)
    }

    /// complaint_clusters_outrank_incident_repeaters_in_capture_priority
    /// (the wired capture leg: kind + salience land on the proposal row).
    #[test]
    fn complaint_capture_outranks_repeater_capture() {
        let mut conn = db();
        // A complaint CLUSTER: three complaint runs inside the window.
        let (r1, _) = seed_closed_case_kind(&conn, "complaint", "{}");
        let (r2, _) = seed_closed_case_kind(&conn, "complaint", "{}");
        let (r3, _) = seed_closed_case_kind(&conn, "complaint", "{}");
        for r in [r1, r2] {
            capture_on_case_close(&mut conn, r, 1_000).unwrap();
        }
        let third = capture_on_case_close(&mut conn, r3, 2_000).unwrap();
        let CaptureOutcome::Proposed(kind, id) = third else {
            panic!("a closed complaint must capture");
        };
        assert_eq!(kind, crate::workflow::complaint::KIND_RCA);
        let salience: f64 = conn
            .query_row(
                "SELECT salience FROM proposals WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(salience, 0.9, "cluster members capture at the top priority");
        // A single complaint (no cluster yet — it is the only complaint run
        // in its window) sits at the repeater tier.
        let (solo, _) = seed_closed_case_kind_at(&conn, "complaint", "{}", 3_000_000);
        if let CaptureOutcome::Proposed(kind_s, id_s) =
            capture_on_case_close(&mut conn, solo, 3_000_001).unwrap()
        {
            assert_eq!(kind_s, crate::workflow::complaint::KIND_RCA);
            let s: f64 = conn
                .query_row(
                    "SELECT salience FROM proposals WHERE id = ?1",
                    params![id_s],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(s, 0.7);
        }
        // A plain interview close keeps the base salience and the plain kinds.
        let (plain, _) = seed_closed_case(&conn, "{}");
        if let CaptureOutcome::Proposed(kind_p, id_p) =
            capture_on_case_close(&mut conn, plain, 4_000).unwrap()
        {
            assert_ne!(kind_p, crate::workflow::complaint::KIND_RCA);
            let p: f64 = conn
                .query_row(
                    "SELECT salience FROM proposals WHERE id = ?1",
                    params![id_p],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(p, 0.5);
        }
    }

    #[test]
    fn sir_rows_record_found_and_not_found() {
        let mut conn = db();
        // An article for the reuse search to find.
        conn.execute(
            "INSERT INTO knowledge(content, title, content_hash, domain) VALUES ('reset pin guide', 'PIN reset', 'h1', 'global')",
            [],
        )
        .unwrap();
        let article = conn.last_insert_rowid();
        let (run_id, case_ref) = seed_closed_case(&conn, r#"{"q":"pin reset"}"#);

        assert_eq!(record_sir_found(&conn, run_id, &[article], 100), 1);
        // Idempotent per (case, article).
        assert_eq!(record_sir_found(&conn, run_id, &[article], 101), 0);
        assert_eq!(record_sir_not_found(&mut conn, run_id, 102), 1);

        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM case_articles WHERE case_ref=?1 AND sir='searched_found'",
                params![case_ref],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(found, 1);
        let not_found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM case_articles WHERE sir='searched_not_found' AND knowledge_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(not_found, 1);
        // A run with no CRM binding records nothing (no orphan rows).
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('global', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        let unbound = conn.last_insert_rowid();
        assert_eq!(record_sir_found(&conn, unbound, &[article], 103), 0);
        assert_eq!(record_sir_not_found(&mut conn, unbound, 104), 0);
    }

    #[test]
    fn improve_flag_emitted_on_cited_article_contradiction() {
        let mut conn = db();
        conn.execute(
            "INSERT INTO knowledge(content, title, content_hash, domain) VALUES ('reboot guide', 'Reboot', 'h2', 'global')",
            [],
        )
        .unwrap();
        let article = conn.last_insert_rowid();
        let (run_id, case_ref) = seed_closed_case(
            &conn,
            r#"{"steps":[{"expected":"article said reboot","actual":"actually re-provision","skipped_verify":true}]}"#,
        );
        record_sir_found(&conn, run_id, &[article], 100);

        let flagged = flag_contradicted_articles(
            &mut conn,
            run_id,
            &serde_json::json!({"contradictions": 1}),
            200,
        );
        assert_eq!(flagged, vec![article]);
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM findings WHERE claim='kcs_flag' AND evidence=?1",
                params![format!("article:{article}")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);

        // A clean completed run flags nothing.
        let clean_state = serde_json::json!({"contradictions": 0});
        assert!(flag_contradicted_articles(&mut conn, run_id, &clean_state, 300).is_empty());
        // No citation → no flag target even with contradictions.
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('global', 'interview', '{\"contradictions\":2}', 0, 'completed', 1, 1)",
            [],
        )
        .unwrap();
        let other = conn.last_insert_rowid();
        let _ = seed_closed_case; // (binding below reuses the helper shape)
        conn.execute(
            "INSERT INTO crm_cases(case_ref, source, org_id, case_id, run_id, status, updated_rev, synced_at)
             VALUES ('crm:salesforce:acme:9', 'salesforce', 'acme', '9', ?1, 'closed_solved', 'r9', CURRENT_TIMESTAMP)",
            params![other],
        )
        .unwrap();
        assert!(
            flag_contradicted_articles(
                &mut conn,
                other,
                &serde_json::json!({"contradictions":2}),
                400
            )
            .is_empty(),
            "no SIR citations means no article to flag"
        );
        let _ = case_ref;
    }

    #[test]
    fn unlinked_closed_case_is_flagged_not_blocked() {
        let mut conn = db();
        let (run_id, case_ref) = seed_closed_case(&conn, r#"{"q":"mystery","is_seed":"symptom"}"#);
        // Zero linkage: capture still proposes (never blocks), and flags.
        match capture_on_case_close(&mut conn, run_id, 500).expect("capture") {
            CaptureOutcome::Proposed(kind, pid) => {
                assert_eq!(kind, KIND_NEW, "unlinked case proposes a NEW article");
                let content: String = conn
                    .query_row(
                        "SELECT content FROM proposals WHERE id=?1",
                        params![pid],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    parse_preamble(&content).map(|(c, _)| c),
                    Some(case_ref.clone())
                );
                for section in [
                    "## Issue",
                    "## Environment",
                    "## Cause",
                    "## Resolution",
                    "## Evidence",
                ] {
                    assert!(content.contains(section), "{section} missing");
                }
            }
            other => panic!("expected a proposal, got {other:?}"),
        }
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM findings WHERE run_id=?1 AND claim='kcs_unlinked_case'",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the gap is visible");
        // The case row itself is untouched (flag, never veto).
        let status: String = conn
            .query_row(
                "SELECT status FROM crm_cases WHERE case_ref=?1",
                params![case_ref],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "closed_solved");

        // Replay is exactly-once: no second proposal.
        assert_eq!(
            capture_on_case_close(&mut conn, run_id, 600).unwrap(),
            CaptureOutcome::Skipped
        );
        let proposals: i64 = conn
            .query_row("SELECT COUNT(*) FROM proposals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(proposals, 1);
    }

    #[test]
    fn capture_branches_follow_the_citation_and_divergence_signals() {
        let mut conn = db();
        // Cited + clean reuse → link_only proposal naming the existing article.
        conn.execute(
            "INSERT INTO knowledge(content, title, content_hash, domain) VALUES ('good guide', 'Guide', 'h3', 'global')",
            [],
        )
        .unwrap();
        let article = conn.last_insert_rowid();
        let (run_id, _) = seed_closed_case(
            &conn,
            r#"{"steps":[{"expected":"x","actual":"x"}],"is_seed":"clean symptom"}"#,
        );
        record_sir_found(&conn, run_id, &[article], 100);
        match capture_on_case_close(&mut conn, run_id, 700).unwrap() {
            CaptureOutcome::Proposed(kind, pid) => {
                assert_eq!(kind, KIND_LINK_ONLY);
                let content: String = conn
                    .query_row(
                        "SELECT content FROM proposals WHERE id=?1",
                        params![pid],
                        |r| r.get(0),
                    )
                    .unwrap();
                let (_, art) = parse_preamble(&content).expect("preamble");
                assert_eq!(art, Some(article));
            }
            other => panic!("expected proposal, got {other:?}"),
        }

        // Cited + diverged steps → update proposal.
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('global', 'interview', '{\"steps\":[{\"expected\":\"a\",\"actual\":\"b\"}]}', 0, 'completed', 1, 1)",
            [],
        )
        .unwrap();
        let run2 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO crm_cases(case_ref, source, org_id, case_id, run_id, status, updated_rev, synced_at)
             VALUES ('crm:genesys:acme:11', 'genesys', 'acme', '11', ?1, 'closed_solved', 'r11', CURRENT_TIMESTAMP)",
            params![run2],
        )
        .unwrap();
        record_sir_found(&conn, run2, &[article], 110);
        match capture_on_case_close(&mut conn, run2, 800).unwrap() {
            CaptureOutcome::Proposed(kind, _) => assert_eq!(kind, KIND_UPDATE),
            other => panic!("expected update proposal, got {other:?}"),
        }

        // An open (not closed) case never captures.
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('global', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        let run3 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO crm_cases(case_ref, source, org_id, case_id, run_id, status, updated_rev, synced_at)
             VALUES ('crm:zendesk:acme:12', 'zendesk', 'acme', '12', ?1, 'open', 'r12', CURRENT_TIMESTAMP)",
            params![run3],
        )
        .unwrap();
        assert_eq!(
            capture_on_case_close(&mut conn, run3, 900).unwrap(),
            CaptureOutcome::Skipped
        );
    }

    #[test]
    fn kcs_measures_are_deterministic_and_zero_safe() {
        let conn = db();
        let m0 = kcs_measures(&conn, 1000).unwrap();
        assert_eq!(
            m0,
            KcsMeasures {
                linkage_rate_units: 0,
                searched_found_rate_units: 0,
                article_freshness_median_age_secs: 0
            },
            "no evidence reads as zero, never as good"
        );
        conn.execute(
            "INSERT INTO crm_cases(case_ref, source, org_id, case_id, run_id, status, updated_rev, synced_at)
             VALUES ('crm:z:a:1','z','a','1',NULL,'closed_solved','r','ts') ,('crm:z:a:2','z','a','2',NULL,'closed_solved','r','ts')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO knowledge(content, title, content_hash, domain, kcs_state, created_at) VALUES ('g','G','h4','global','draft',500)",
            [],
        )
        .unwrap();
        let art = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO case_articles(case_ref, knowledge_id, sir, action, ts) VALUES ('crm:z:a:1', ?1, 'searched_found', 'linked', 1)",
            params![art],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO case_articles(case_ref, knowledge_id, sir, action, ts) VALUES ('crm:z:a:2', NULL, 'searched_not_found', 'linked', 1)",
            [],
        )
        .unwrap();
        let m = kcs_measures(&conn, 1500).unwrap();
        assert_eq!(m.linkage_rate_units, 5000, "one of two closed cases linked");
        assert_eq!(
            m.searched_found_rate_units, 5000,
            "one found of one found+one not-found"
        );
        assert_eq!(m.article_freshness_median_age_secs, 1000);
    }

    // ── Keystone M2: the governed translation lifecycle.

    fn insert_article(conn: &Connection, content: &str, hash: &str) -> i64 {
        conn.execute(
            "INSERT INTO knowledge(content, source, content_hash, kcs_state) VALUES (?1, 'manual', ?2, 'published')",
            rusqlite::params![content, hash],
        ).expect("insert article");
        conn.last_insert_rowid()
    }

    #[test]
    fn translate_proposal_never_autopopulates() {
        let conn = db();
        let id = insert_article(&conn, "art", "h1");
        // Filing a proposal creates a PENDING proposal and NO approved row.
        let pid = propose_translation(
            &conn,
            &TranslationDraft {
                knowledge_id: id,
                locale: "de",
                title: "Titel",
                body_md: "Inhalt",
                translator: "maria",
            },
            1000,
        )
        .expect("file");
        let (kind, status): (String, String) = conn
            .query_row(
                "SELECT kind, status FROM proposals WHERE id = ?1",
                params![pid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("proposal");
        assert_eq!(kind, KIND_TRANSLATE);
        assert_eq!(status, "pending");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM kcs_translations", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 0, "nothing auto-populates the translations table");
        // Only approval writes — through the single gate path.
        let content: String = conn
            .query_row(
                "SELECT content FROM proposals WHERE id = ?1",
                params![pid],
                |r| r.get(0),
            )
            .expect("content");
        let tr = apply_translation_approval(&conn, &content, 2000).expect("apply");
        assert!(tr > 0);
        let (state, based): (String, String) = conn
            .query_row("SELECT state, based_revision FROM kcs_translations WHERE knowledge_id = ?1 AND locale = 'de'", params![id], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("translation");
        assert_eq!(state, "approved");
        assert_eq!(based, "h1", "pinned to the source revision at approval");
        // Bounds + validation refuse loudly.
        assert!(
            propose_translation(
                &conn,
                &TranslationDraft {
                    knowledge_id: id,
                    locale: "../evil",
                    title: "t",
                    body_md: "b",
                    translator: "x"
                },
                1
            )
            .is_err()
        );
        assert!(
            propose_translation(
                &conn,
                &TranslationDraft {
                    knowledge_id: 999_999,
                    locale: "de",
                    title: "t",
                    body_md: "b",
                    translator: "x"
                },
                1
            )
            .is_err()
        );
    }

    #[test]
    fn translation_goes_stale_when_source_revision_advances() {
        let conn = db();
        let id = insert_article(&conn, "art v1", "hash-v1");
        propose_translation(
            &conn,
            &TranslationDraft {
                knowledge_id: id,
                locale: "fr",
                title: "T",
                body_md: "B",
                translator: "x",
            },
            1000,
        )
        .expect("file");
        let content: String = conn
            .query_row(
                "SELECT content FROM proposals WHERE kind = ?1",
                params![KIND_TRANSLATE],
                |r| r.get(0),
            )
            .expect("content");
        apply_translation_approval(&conn, &content, 2000).expect("apply");
        // Fresh: based_revision matches.
        assert!(stale_translations(&conn).expect("fresh check").is_empty());
        // The source advances (an edit → new content_hash).
        conn.execute(
            "UPDATE knowledge SET content = 'art v2', content_hash = 'hash-v2' WHERE id = ?1",
            params![id],
        )
        .expect("advance");
        let stale = stale_translations(&conn).expect("stale check");
        assert_eq!(
            stale.len(),
            1,
            "the advanced source lands the translation on the worklist"
        );
        assert_eq!(stale[0]["locale"], "fr");
        assert_eq!(stale[0]["kind"], "translation_stale");
    }
}
