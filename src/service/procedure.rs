//! The procedure aggregate: procedural memory + its ordered step chains.
//!
//! OWNS the procedure family's storage story:
//! - the procedure store tx — root chunk (`node_kind='procedure'`) →
//!   per-chunk quarantine flags → ordered step chunks (`parent_id` children
//!   of the root, FK-children order: root first) → `next_step` edges
//!   (skipped entirely for a quarantined root so a flagged plant cannot
//!   reach the graph), all inside the CALLER'S tx;
//! - the ordered step-chain read (domain-label bound — an id cannot cross
//!   domains), the row-meta read the handler re-authorizes on, and the
//!   decision-rule read;
//! - the vec-shadow writes (`vec_knowledge`) and the per-chunk content
//!   reads the embedding pass needs — best-effort by contract, mirroring
//!   the /ingest path's tolerance.
//!
//! FK-children map: step rows are `knowledge.parent_id` children of the
//! root; `evidence_links.next_step` edges reference both. This aggregate
//! has NO delete path; erasure of procedure chunks flows through the purge
//! core, which owns the family's delete ordering.
//!
//! The screen verdicts and their wire shapes stay at the handler (the
//! body-scan pin holds `screen::screen(` inside the handler's `create`);
//! this core receives the already-computed quarantine flags and owns the
//! flag writes + the edge-skip rule. Error Display carries the exact
//! pre-move message text — the handler renders it verbatim as a 500.

use std::fmt;

use rusqlite::{Connection, OptionalExtension, params};

use crate::procedural::MemoryKind;

/// One ordered step-chain row: (id, title, content, node_kind, step_index).
pub(crate) type StepChainRow = (i64, Option<String>, String, String, Option<i64>);

/// A chunk's access meta: (domain, owner, access_scope).
pub(crate) type AccessMeta = (String, Option<String>, Option<String>);

/// A storage failure. `Display` carries the exact pre-move message; the
/// handler wraps it in `HandlerError::internal` unchanged.
#[derive(Debug)]
pub(crate) enum ProcedureError {
    Storage(String),
}

impl fmt::Display for ProcedureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcedureError::Storage(m) => f.write_str(m),
        }
    }
}

impl From<rusqlite::Error> for ProcedureError {
    fn from(e: rusqlite::Error) -> Self {
        ProcedureError::Storage(e.to_string())
    }
}

/// Store a procedure root + its ordered steps inside the CALLER'S tx.
///
/// Per-chunk contract: every chunk (root and each step) is flagged iff its
/// screen verdict quarantined (fail-closed — a step that must be flagged
/// but can't be aborts the tx); the `next_step` edges are written only when
/// the ROOT is clean, and their `ON CONFLICT DO UPDATE` makes a re-ingest
/// of the same pair idempotent. `domain` defaults to `global`; `origin` is
/// `operator`, `source` `manual` — both verbatim.
pub(crate) fn store_procedure(
    tx: &rusqlite::Transaction<'_>,
    title: &str,
    content: &str,
    domain: Option<&str>,
    steps: &[(String, String, MemoryKind)],
    root_quarantine: bool,
    step_quarantine: &[bool],
) -> Result<(i64, Vec<i64>), ProcedureError> {
    // Root chunk: memory_kind = 'procedure'.
    let content_hash = crate::audit::hash(&format!("{title}|{content}"));
    tx.execute(
        "INSERT INTO knowledge (title, content, content_hash, source, domain, node_kind, origin)
         VALUES (?1, ?2, ?3, 'manual', ?4, 'procedure', 'operator')",
        params![title, content, content_hash, domain.unwrap_or("global")],
    )
    .map_err(|e| ProcedureError::Storage(format!("procedure insert failed: {e}")))?;
    let root_id = tx.last_insert_rowid();
    // flag the root if the screen quarantined. Excluded from
    // recall via `WHERE flagged = 0`, KG edges skipped below.
    let root_flagged = crate::flag_if_quarantined(tx, root_id, root_quarantine)
        .map_err(|e| ProcedureError::Storage(format!("quarantine flag failed: {e}")))?;
    let mut step_ids: Vec<i64> = Vec::new();
    for (idx, (step_title, step_content, step_kind)) in steps.iter().enumerate() {
        let hash = crate::audit::hash(&format!("{root_id}|{idx}|{step_title}|{step_content}"));
        let kind_str = step_kind.as_str();
        tx.execute(
            "INSERT INTO knowledge (title, content, content_hash, source, domain, node_kind, parent_id, origin)
             VALUES (?1, ?2, ?3, 'manual', ?4, ?5, ?6, 'operator')",
            params![
                step_title,
                step_content,
                hash,
                domain.unwrap_or("global"),
                kind_str,
                root_id
            ],
        )
        .map_err(|e| ProcedureError::Storage(format!("step insert failed: {e}")))?;
        let step_id = tx.last_insert_rowid();
        // flag this step if its screen quarantined. The verdict
        // was computed per-step before the tx, so only the steps that
        // actually quarantined are flagged (a benign step in a quarantined
        // procedure stays clean). Fails closed: a step that must be
        // flagged but can't be is a stop-the-write condition.
        crate::flag_if_quarantined(tx, step_id, step_quarantine[idx])
            .map_err(|e| ProcedureError::Storage(format!("quarantine flag failed: {e}")))?;
        // next_step edge with explicit ordering. Skipped for a quarantined
        // root so a flagged plant can't reach the graph even via a step.
        // The edge kind is 'next_step'; step_index carries the position.
        // UNIQUE(from_chunk, to_chunk, kind) means a re-ingest of the same
        // pair is idempotent.
        if !root_flagged {
            tx.execute(
                "INSERT INTO evidence_links (from_chunk, to_chunk, kind, step_index)
                 VALUES (?1, ?2, 'next_step', ?3)
                 ON CONFLICT(from_chunk, to_chunk, kind) DO UPDATE SET step_index = ?3",
                params![root_id, step_id, idx as i64],
            )
            .map_err(|e| ProcedureError::Storage(format!("next_step edge failed: {e}")))?;
        }
        step_ids.push(step_id);
    }
    Ok((root_id, step_ids))
}

/// Stored content of one chunk (the embedding pass's per-step read).
pub(crate) fn chunk_content(conn: &Connection, chunk_id: i64) -> rusqlite::Result<String> {
    conn.query_row(
        "SELECT content FROM knowledge WHERE id = ?1",
        params![chunk_id],
        |r| r.get::<_, String>(0),
    )
}

/// Best-effort vec-shadow write: int8 + bit quantization over the same
/// f32→bytes cast the /ingest path uses. The CALLER owns the best-effort
/// posture (a failure here must not undo a committed store — the FTS5
/// shadow row keeps the chunk retrievable without a vector).
pub(crate) fn store_embedding(
    conn: &Connection,
    chunk_id: i64,
    embedding_bytes: &[u8],
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT OR REPLACE INTO vec_knowledge (knowledge_id, embedding_int8, embedding_bit, source, created_at)
         VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), 'manual', datetime('now'))",
        params![chunk_id, embedding_bytes],
    )
}

/// The procedure root's (title, content) — domain-label bound AND
/// node-kind bound, so a bare id cannot read a non-procedure chunk.
pub(crate) fn procedure_root(
    conn: &Connection,
    id: i64,
    domain_label: &str,
) -> rusqlite::Result<Option<(Option<String>, String)>> {
    conn.query_row(
        "SELECT title, content FROM knowledge \
         WHERE id = ?1 AND node_kind = 'procedure' AND domain = ?2",
        params![id, domain_label],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

/// The row's own access meta (domain, owner, access_scope) — the input to
/// the handler's belt-and-braces re-authorization + record gate. The GATE
/// DECISION stays handler-side.
pub(crate) fn row_access_meta(conn: &Connection, id: i64) -> rusqlite::Result<Option<AccessMeta>> {
    conn.query_row(
        "SELECT domain, owner, access_scope FROM knowledge WHERE id = ?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
}

/// The ordered step chain via the `next_step` edges, domain-label bound
/// (steps live with their procedure). Row errors skip — the handler's
/// filter_map posture, verbatim.
pub(crate) fn step_chain(
    conn: &Connection,
    procedure_id: i64,
    domain_label: &str,
) -> Result<Vec<StepChainRow>, ProcedureError> {
    let mut stmt = conn
        .prepare(
            "SELECT k.id, k.title, k.content, k.node_kind, el.step_index \
             FROM evidence_links el \
             JOIN knowledge k ON k.id = el.to_chunk \
             WHERE el.from_chunk = ?1 AND el.kind = 'next_step' AND k.domain = ?2 \
             ORDER BY el.step_index ASC",
        )
        .map_err(|e| ProcedureError::Storage(format!("prepare failed: {e}")))?;
    let rows = stmt
        .query_map(params![procedure_id, domain_label], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(|e| ProcedureError::Storage(format!("query failed: {e}")))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// The stored decision rule content of a `decision`-kind chunk. The
/// no-rows vs other-error split stays at the handler (404 vs 500 mapping).
pub(crate) fn decision_rule_content(conn: &Connection, id: i64) -> rusqlite::Result<String> {
    conn.query_row(
        "SELECT content FROM knowledge WHERE id = ?1 AND node_kind = 'decision'",
        params![id],
        |r| r.get(0),
    )
}
