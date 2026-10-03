//! The drift census's storage layer: a census breach becomes an **auditable row**,
//! not a log line nobody reads.
//!
//! ## What this core owns
//!
//! Every statement that writes or reads a census row, the bounds that guard
//! them, and the hash-chained audit row each mutation owes — written INSIDE the
//! caller's transaction, so a breach and its evidence commit or roll back
//! together. The comparison itself is not here: it is
//! [`crate::workflow::drift_census`], which is pure.
//!
//! ## The mapping onto `findings`, stated because it is lossy
//!
//! The repository already has a first-class findings table (`migration.rs:1697`:
//! `id, run_id, claim, evidence, source, confidence REAL, ts`), and this round
//! writes census breaches **there** rather than inventing a second findings
//! concept. But a census breach is a *cell regression with a delta*, and that
//! table was shaped for a *run observation with a confidence*. Three mappings
//! are honest only if they are said out loud:
//!
//! | Column | What a breach puts there | What that costs |
//! |---|---|---|
//! | `run_id` | `CENSUS_RUN_ID` (`0`) | **The census has no run.** `webhook_ingest.rs:47` already writes `run_id = 0` as a run-less sentinel, so this is the *second* writer under that convention — named, pinned, and disclosed rather than left to be rediscovered. |
//! | `claim` | the cell id | The column is `TEXT`, so a cell id is a legal claim. Nothing is lost. |
//! | `evidence` | `cell=<id> baseline=<n> observed=<n> delta=<n> tolerance=<n>` | **A parsed, closed grammar, not prose.** The magnitude rides here because the table has no numeric column for it — that is the real lossiness, and it is why the format is pinned rather than left to drift. |
//! | `confidence` | `1.0` | **This is the lossy one.** The column is a *ranking weight* for the SDK's evidence reducer, not a calibrated probability, and a regression is a measured fact rather than a belief. `1.0` is the same "this is certain" reading `record_kb_feedback_finding` already uses. It does **not** encode severity: a 600-unit drift and a 4000-unit drift both write `1.0`, and the magnitude is only in `evidence`. Said here so no reader mistakes it for a probability. |
//! | `source` | [`CENSUS_SOURCE`] | **`source` has no closed vocabulary in this schema** — the only literal in the tree is `"gdl"`, in tests. This core therefore *establishes* a convention rather than following one, and pins the single value it writes. |
//!
//! ## Why the audit row is in-tx, and why it is `Workflow`
//!
//! The write is a governed-workflow mutation of an evidence table, which is
//! exactly what `AuditKind::Workflow` already covers ("every workflow
//! run/step/outbox/finding/contradiction mutation hash-chains a row so the
//! evidence tables are derivable from the audit"). It rides **inside** the
//! caller's transaction: a breach row whose audit row committed separately is a
//! breach nobody can attribute, which is the same unevidenced-write window the
//! Foundation Line closed. This core deliberately does **not** copy
//! `record_kb_feedback_finding`'s autocommit-after-the-write posture, which its
//! own module header declares as the exception.
//!
//! ## What this core does not own
//!
//! **It does not decide anything.** No tolerance lives here, and no verdict is
//! computed here. A storage layer that also holds the threshold is a second,
//! unreviewed copy of the policy — and the whole point of `P57.2` is that there
//! is exactly one tolerance, in one place, and it is not tunable from a call
//! site.

use rusqlite::{Connection, params};

use crate::audit::{AuditKind, AuditStatus};
use crate::workflow::drift_census::{Census, GLOBAL_TOLERANCE_UNITS};

/// The `findings.source` value a census breach writes.
///
/// **`source` has no closed vocabulary in this schema.** This is a new
/// convention, declared here and pinned by
/// `tests::the_census_writes_exactly_one_source_value` so a second writer cannot
/// drift the spelling unnoticed.
pub(crate) const CENSUS_SOURCE: &str = "drift_census";

/// The run-less sentinel every census row is written under.
///
/// A census is not a case: it measures the corpus, not a run, so there is no
/// `workflow_runs` row to key it to. `run_id = 0` is the sentinel
/// `webhook_ingest.rs` already established, and it is **not** a foreign key —
/// the column carries no FK, so `0` is a legal stored value and reads as "no
/// run" rather than as a broken reference.
///
/// This is the SECOND writer under that convention and it is named rather than
/// incidental: a reader that filters census rows must select on `source`, not on
/// `run_id`, or it will pick up the webhook's rows too.
pub(crate) const CENSUS_RUN_ID: i64 = 0;

/// The bound on the evidence string. Every field is a closed token or a bounded
/// integer, so this is a tripwire against a future contributor routing a body
/// into it — the census's evidence is a measurement, and a measurement has a
/// fixed size.
pub(crate) const MAX_EVIDENCE_BYTES: usize = 256;

/// The bound on rows one census may write, matching the census's own case bound.
pub(crate) const MAX_BREACH_ROWS: usize = 64;

/// What one census breach row costs, as the table's `confidence` column reads it.
///
/// **Not a probability.** The column is a ranking weight in the SDK's evidence
/// reducer (`evidence.rs` sorts by it), and a regression past tolerance is a
/// measured fact rather than a belief — the same "certain" reading
/// `record_kb_feedback_finding` uses. Severity does NOT ride here: a 600-unit
/// drift and a 4000-unit drift both write this value, and the magnitude is in
/// `evidence`. Stated here so no reader mistakes it for calibration.
pub(crate) const BREACH_CONFIDENCE: f64 = 1.0;

/// One census breach, reduced to what a row carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BreachRow {
    /// The census cell that moved.
    pub(crate) cell: String,
    /// What the committed baseline recorded.
    pub(crate) baseline_units: i32,
    /// What this run measured.
    pub(crate) observed_units: i32,
    /// Signed movement, in the scorer's integer ten-thousandths.
    pub(crate) delta_units: i32,
}

/// The audit target every census breach writes under. It is a module constant
/// rather than a literal at the call site so the hash the chain stores and the
/// value a reader joins on cannot drift apart.
pub(crate) const AUDIT_TARGET: &str = "drift_census";

/// The core's typed error. `Display` preserves the underlying message so the
/// caller can map it onto its own vocabulary; the core never names an HTTP
/// status, a wire code, or an exit code.
#[derive(Debug)]
pub(crate) enum CensusError {
    Storage(String),
    /// The breach count is past the bound. Refused, never truncated: a census
    /// that silently wrote the first 64 breaches would report a clean tail it
    /// never measured.
    TooManyBreaches(usize),
}

impl std::fmt::Display for CensusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CensusError::Storage(m) => write!(f, "storage: {m}"),
            CensusError::TooManyBreaches(n) => {
                write!(
                    f,
                    "census holds {n} breaches, past the bound of {MAX_BREACH_ROWS}"
                )
            }
        }
    }
}

impl From<rusqlite::Error> for CensusError {
    fn from(e: rusqlite::Error) -> Self {
        CensusError::Storage(e.to_string())
    }
}

/// The closed evidence grammar a breach row carries.
///
/// Every field is a closed token or a bounded integer, and the whole string is
/// bounded by [`MAX_EVIDENCE_BYTES`]. **This is prose-free on purpose**: the
/// magnitude has to be recoverable from the row, because the table has no
/// numeric column for it, and a message an operator must parse is a message
/// they will not. The format is pinned by
/// `tests::the_evidence_grammar_round_trips_every_field`.
pub(crate) fn render_evidence(row: &BreachRow) -> String {
    format!(
        "cell={} baseline={} observed={} delta={} tolerance={}",
        row.cell, row.baseline_units, row.observed_units, row.delta_units, GLOBAL_TOLERANCE_UNITS
    )
}

/// Reduce a census's breaching cells to the rows they would write.
///
/// Pure, and the single place the reduction happens — so a caller cannot widen
/// or narrow what counts as a breach without passing this function.
pub(crate) fn breach_rows(census: &Census) -> Result<Vec<BreachRow>, CensusError> {
    let breaches = census.breaches();
    if breaches.len() > MAX_BREACH_ROWS {
        return Err(CensusError::TooManyBreaches(breaches.len()));
    }
    let mut out = Vec::with_capacity(breaches.len());
    for result in breaches {
        // A breach ALWAYS carries a delta: `CellVerdict::Drifted` is reachable
        // only through the baselined arm of the census, which is the one place
        // `delta_units` is set. The refusal is structural defence, not a branch
        // a future edit is expected to take.
        let Some(delta) = result.delta_units else {
            return Err(CensusError::Storage(
                "a drifted cell carries no delta; the census and this reduction disagree".into(),
            ));
        };
        out.push(BreachRow {
            cell: result.id.clone(),
            baseline_units: result.observed_units.saturating_sub(delta),
            observed_units: result.observed_units,
            delta_units: delta,
        });
    }
    Ok(out)
}

/// Write the census breach rows and ONE audit row, inside the caller's
/// transaction.
///
/// The audit detail is **closed vocabulary plus numbers** and carries no cell
/// content beyond the cell's own id, which is a corpus case name and therefore
/// already public. It names the count and the worst movement, so an operator
/// reading the chain learns that a breach happened without the chain becoming a
/// place where claim content accumulates.
pub(crate) fn record_breaches(
    tx: &rusqlite::Transaction<'_>,
    rows: &[BreachRow],
    corpus_id: &str,
    now: i64,
) -> Result<usize, CensusError> {
    if rows.is_empty() {
        // A clean census writes nothing at all. A census that wrote an audit row
        // every time it found nothing would bury the breaches in green noise,
        // and a green row nobody reads is the failure this round exists to fix.
        return Ok(0);
    }
    if rows.len() > MAX_BREACH_ROWS {
        return Err(CensusError::TooManyBreaches(rows.len()));
    }
    for row in rows {
        if row.cell.is_empty() || row.cell.len() > crate::workflow::drift_census::MAX_CELL_ID_BYTES
        {
            return Err(CensusError::Storage("census cell id out of bounds".into()));
        }
        let evidence = render_evidence(row);
        if evidence.len() > MAX_EVIDENCE_BYTES {
            return Err(CensusError::Storage(
                "census evidence exceeds its bound".into(),
            ));
        }
        tx.execute(
            "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                CENSUS_RUN_ID,
                row.cell,
                evidence,
                CENSUS_SOURCE,
                BREACH_CONFIDENCE,
                now,
            ],
        )?;
    }
    // The worst movement, so the chain carries a magnitude without carrying the
    // set. A reader who wants every cell reads the findings rows the chain
    // points at; a reader who wants "did anything move and how far" reads this.
    let worst = rows
        .iter()
        .map(|r| i64::from(r.delta_units).abs())
        .max()
        .unwrap_or(0);
    crate::audit::record_tenant(
        tx,
        AuditKind::Workflow,
        crate::workflow::ACTOR,
        AUDIT_TARGET,
        AuditStatus::Ok,
        &format!(
            "census breach: {n} cell(s) past tolerance {GLOBAL_TOLERANCE_UNITS}, worst {worst} units, corpus {corpus_id}",
            n = rows.len()
        ),
        "global",
    );
    Ok(rows.len())
}

/// Read the census rows this core wrote, newest first.
///
/// Bounded and probe-shaped: a caller cannot ask it for another source's rows,
/// so a census reader can never widen into the evidence table. `limit` is
/// clamped, never trusted.
pub(crate) fn list_breaches(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<(String, String, i64)>, CensusError> {
    let limit = limit.clamp(1, MAX_BREACH_ROWS) as i64;
    let mut stmt = conn.prepare(
        "SELECT claim, evidence, ts FROM findings
          WHERE source = ?1
          ORDER BY id DESC
          LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![CENSUS_SOURCE, limit], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::workflow::drift_census::{Cell, census as run_census};
    use std::collections::BTreeMap;

    fn db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migration(&mut conn, 512).expect("migration");
        conn
    }

    fn base(pairs: &[(&str, i32)]) -> BTreeMap<String, i32> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    /// The frozen human verdict per case id, as the census receives it.
    fn verdicts(pairs: &[(&str, bool)]) -> BTreeMap<String, bool> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn a_planted_regression_becomes_a_findings_row_and_an_audit_row() {
        let conn = db();
        // The plant: `qc_clean_run` is baselined at 10 000 and now measures
        // 5 000, which is five thousand units past the global tolerance.
        let cells = vec![Cell {
            id: "qc_clean_run".into(),
            observed_units: 5_000,
        }];
        let out = run_census(
            &cells,
            &base(&[("qc_clean_run", 10_000)]),
            &verdicts(&[("qc_clean_run", true)]),
        );
        assert!(out.breaches().len() == 1, "the plant must breach");

        let tx = conn.unchecked_transaction().expect("tx");
        let rows = breach_rows(&out).expect("reduce");
        let n = record_breaches(&tx, &rows, "gold-sets", 1_700_000_000).expect("write");
        tx.commit().expect("commit");
        assert_eq!(n, 1);

        let stored: (String, String, String, f64) = conn
            .query_row(
                "SELECT claim, evidence, source, confidence FROM findings WHERE run_id = ?1",
                params![CENSUS_RUN_ID],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .expect("row");
        assert_eq!(stored.0, "qc_clean_run");
        assert_eq!(stored.2, CENSUS_SOURCE);
        assert_eq!(stored.3, BREACH_CONFIDENCE);
        assert_eq!(
            stored.1, "cell=qc_clean_run baseline=10000 observed=5000 delta=-5000 tolerance=500",
            "the evidence grammar must name every field, so a reader can recover the magnitude \
             from the row — the table has no numeric column for it"
        );

        // And the row is HASH-CHAINED. `audit_events` stores only the HASH of a
        // target (`target_hash`), never the plaintext — so the census's audit
        // row is joined by `hash("drift_census")`, which is the same join every
        // other audit-derived surface in this repository makes. The row and its
        // evidence share a transaction, so they commit or roll back together.
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE target_hash = ?1",
                params![crate::audit::hash(AUDIT_TARGET)],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(
            audits, 1,
            "a breach row without its audit row is unevidenced"
        );
    }

    #[test]
    fn a_clean_census_writes_nothing_at_all() {
        let conn = db();
        let out = run_census(
            &[Cell {
                id: "qc_clean_run".into(),
                observed_units: 10_000,
            }],
            &base(&[("qc_clean_run", 10_000)]),
            &verdicts(&[("qc_clean_run", true)]),
        );
        assert!(out.is_clean());
        let tx = conn.unchecked_transaction().expect("tx");
        let n =
            record_breaches(&tx, &breach_rows(&out).expect("reduce"), "gold-sets", 1).expect("w");
        tx.commit().expect("commit");
        assert_eq!(n, 0, "a clean census must not write a row");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM findings", [], |r| r.get(0))
            .expect("count");
        assert_eq!(
            rows, 0,
            "green rows are noise; a breach buried in them is unread"
        );
    }

    #[test]
    fn the_breach_row_and_its_audit_roll_back_together() {
        let conn = db();
        let out = run_census(
            &[Cell {
                id: "qc_clean_run".into(),
                observed_units: 0,
            }],
            &base(&[("qc_clean_run", 10_000)]),
            &verdicts(&[("qc_clean_run", true)]),
        );
        let tx = conn.unchecked_transaction().expect("tx");
        record_breaches(&tx, &breach_rows(&out).expect("reduce"), "gold-sets", 1).expect("w");
        // Roll back instead of committing: the whole point of the in-tx audit is
        // that a breach and its evidence share a fate.
        drop(tx);
        let (rows, audits): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM findings), (SELECT COUNT(*) FROM audit_events)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("count");
        assert_eq!(
            (rows, audits),
            (0, 0),
            "the rollback took the evidence with it"
        );
    }

    #[test]
    fn an_unbaselined_cell_writes_no_row() {
        // Nothing regressed; something was never measured. Filing that as a
        // regression would put a fabricated finding into an evidence table.
        let out = run_census(
            &[Cell {
                id: "brand_new_case".into(),
                observed_units: 10_000,
            }],
            &BTreeMap::new(),
            &verdicts(&[("brand_new_case", true)]),
        );
        assert_eq!(breach_rows(&out).expect("reduce").len(), 0);
    }

    #[test]
    fn the_census_writes_exactly_one_source_value() {
        // `source` has no closed vocabulary in this schema, so this core
        // ESTABLISHES one. Pinning it stops a second writer drifting the
        // spelling, which would make the census's rows unreadable by join.
        assert_eq!(CENSUS_SOURCE, "drift_census");
        let conn = db();
        let tx = conn.unchecked_transaction().expect("tx");
        tx.execute(
            "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts)
             VALUES (0,'c','e','drift_census',1.0,1)",
            [],
        )
        .expect("insert");
        tx.commit().expect("commit");
        let distinct: i64 = conn
            .query_row("SELECT COUNT(DISTINCT source) FROM findings", [], |r| {
                r.get(0)
            })
            .expect("count");
        assert_eq!(distinct, 1);
        assert_eq!(list_breaches(&conn, 10).expect("list").len(), 1);
    }

    #[test]
    fn the_breach_reader_cannot_widen_into_another_source() {
        let conn = db();
        let tx = conn.unchecked_transaction().expect("tx");
        // A webhook row under the same run-less sentinel: the census reader
        // must not return it.
        tx.execute(
            "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts)
             VALUES (0,'kb_feedback','slug','kb_feedback',1.0,1)",
            [],
        )
        .expect("insert");
        tx.commit().expect("commit");
        let got = list_breaches(&conn, 50).expect("list");
        assert!(
            got.is_empty(),
            "the census reader returned another source's row. Both writers use run_id = 0, so \
             selecting on the run sentinel alone would mix the evidence tables."
        );
    }

    #[test]
    fn the_evidence_grammar_carries_every_field_and_no_body() {
        let row = BreachRow {
            cell: "qc_clean_run".into(),
            baseline_units: 10_000,
            observed_units: 4_999,
            delta_units: -5_001,
        };
        let s = render_evidence(&row);
        assert!(s.len() <= MAX_EVIDENCE_BYTES);
        for field in ["cell=", "baseline=", "observed=", "delta=", "tolerance="] {
            assert!(s.contains(field), "the evidence grammar dropped `{field}`");
        }
        // Closed tokens and integers only: a message an operator has to parse is
        // a message they will not, and a body in here would be claim content in
        // an evidence table.
        assert!(
            s.split(' ')
                .all(|kv| kv.split_once('=').is_some_and(|(k, v)| {
                    !k.is_empty()
                        && !v.is_empty()
                        && k.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                        && v.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                })),
            "the evidence grammar emitted a non-token field: {s}"
        );
    }

    #[test]
    fn a_breach_count_past_the_bound_is_refused_not_truncated() {
        let mut cells = Vec::new();
        let mut pairs = Vec::new();
        for i in 0..=MAX_BREACH_ROWS {
            cells.push(Cell {
                id: format!("cell{i}"),
                observed_units: 0,
            });
            pairs.push((format!("cell{i}"), 10_000));
        }
        let out = run_census(
            &cells,
            &base(
                &pairs
                    .iter()
                    .map(|(k, v)| (k.as_str(), *v))
                    .collect::<Vec<_>>(),
            ),
            &BTreeMap::new(),
        );
        assert!(
            breach_rows(&out).is_err(),
            "a census past the row bound must be refused. Truncating would report a clean tail \
             the census never measured."
        );
    }

    #[test]
    fn the_core_holds_no_tolerance_and_no_verdict() {
        // The whole point of P57.2: one tolerance, in one place, not tunable from
        // a call site. A storage layer that also held the threshold would be a
        // second, unreviewed copy of the policy.
        let source = include_str!("drift_census.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or_default();
        assert!(
            !production.contains("const GLOBAL_TOLERANCE"),
            "the storage core must not define a tolerance; P57.2 puts it in the pure census"
        );
        assert!(
            !production.contains("fn census("),
            "the storage core must not re-implement the comparison; a second policy in the layer \
             that persists is a second authority"
        );
    }
}
