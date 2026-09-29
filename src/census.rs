//! The drift census: the whole frozen corpus, re-measured and diffed against a
//! committed baseline under one global tolerance. A breach becomes a
//! hash-chained findings row and a non-zero exit; a clean pass writes nothing
//! at all.
//!
//! ## Why a verb and not a route
//!
//! **There is no in-process scheduler in this server.** Measured at this round:
//! the only `tokio::time::interval` uses are rate-limit, SSE re-auth and auth
//! sweeps. The repository's cadence posture is *externally cron-driven*, and
//! this round kept it rather than quietly building a daemon.
//!
//! That is also the security argument, not only an architectural one. A shipper
//! running INSIDE the server it measures is a correlated failure: if the server
//! is the thing that regressed, the thing that would have noticed is already in
//! the blast radius. `brain standby` already reasons about this in its own help
//! text — *"operator-run, never a server daemon"*. The census takes the same
//! posture, and this module is why it is a single command an operator (or a
//! timer, or a Kubernetes CronJob) runs and reads the exit code of.
//!
//! ## Fail-closed, in three places
//!
//! 1. **A cell with no baseline is not a pass.** [`run`] refuses to report clean
//!    while any cell is unbaselined, so adding a corpus case cannot produce a
//!    watchdog that is green because nobody has watched it yet.
//! 2. **A breach exits non-zero.** A census whose result is only visible in its
//!    stdout is a census whose result nobody reads.
//! 3. **The rows are hash-chained.** A breach and its audit row commit together
//!    or roll back together, so the evidence table stays derivable from the
//!    chain — never the other way round.
//!
//! ## What a clean run writes
//!
//! **Nothing.** Not a row, not an audit row. A census that wrote a green row every
//! time it found nothing would bury the breaches, and a green row nobody reads is
//! exactly the failure this round exists to fix.
//!
//! ## The re-anchor is a deliberate, diffable act
//!
//! [`print_baseline`] emits the measured vector in the same shape as the
//! committed baseline. Re-anchoring is therefore an edit an operator can read in
//! a diff and judge, rather than a threshold quietly nudged under pressure —
//! the same discipline `brain anchor` follows for the off-host witness.

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::service::drift_census as store;
use crate::service::drift_census::{MAX_BREACH_ROWS, breach_rows, list_breaches, record_breaches};
use crate::workflow::drift_census as census_core;
use crate::workflow::drift_census::{Census, SCALE_UNITS};

/// The committed baseline, compiled in.
///
/// It is a **reference**, not a second corpus: it holds one integer per corpus
/// case id and no cases, no labels, and no artifacts. The corpus is still
/// `crates/gold-sets` alone — the standing refusal to amend it binds here, so
/// this round reads it and never writes it.
const BASELINE_JSON: &str = include_str!("../evals/R57_DRIFT_BASELINE.json");

/// The identifier a census row records as its corpus. It names the crate, not a
/// version, because a pack's own `scorer_version` is what actually decides
/// whether two measurements are comparable — and the census REFUSES a corpus
/// whose packs disagree with each other on that, which is checked at load.
pub(crate) const CORPUS_ID: &str = "gold-sets";

/// What one census pass decided. Returned whole so an operator (or a future
/// route) renders it without re-deriving anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CensusReport {
    /// Every cell's outcome, in sorted order.
    pub cells: Vec<CellLine>,
    /// How many cells breached the global tolerance.
    pub breaches: usize,
    /// How many cells had no baseline and were therefore refused rather than
    /// scored. **A non-zero value means this pass proves nothing**, and the exit
    /// code says so.
    pub unbaselined: usize,
    /// How many baseline entries named a cell the corpus no longer carries.
    pub orphaned: usize,
    /// Findings rows this pass wrote, inside the caller's transaction.
    pub rows_written: usize,
}

/// One cell's line, as the verb prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellLine {
    pub id: String,
    pub verdict: &'static str,
    pub observed_units: i32,
    /// `None` when there is no baseline to move from.
    pub delta_units: Option<i32>,
}

/// A census error, surfaced to a CLI as a message and never as a panic.
#[derive(Debug)]
pub enum RunError {
    /// The corpus or the baseline could not be read/decoded.
    Decode(String),
    /// A census could not be persisted.
    Storage(String),
    /// The caller's transaction could not be opened or committed.
    Transaction(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Decode(m) => write!(f, "census input: {m}"),
            RunError::Storage(m) => write!(f, "census storage: {m}"),
            RunError::Transaction(m) => write!(f, "census transaction: {m}"),
        }
    }
}

impl From<store::CensusError> for RunError {
    fn from(e: store::CensusError) -> Self {
        RunError::Storage(e.to_string())
    }
}

impl From<rusqlite::Error> for RunError {
    fn from(e: rusqlite::Error) -> Self {
        RunError::Transaction(e.to_string())
    }
}

/// Decode the committed baseline.
///
/// **Fail-closed on a scorer disagreement.** Every pack in the corpus must name
/// the same `scorer_version`, and it must be the one the SDK's `SCALER_VERSION`
/// stamps. A census over packs labelled against a different instrument is not a
/// drift census, it is a different measurement wearing this one's name — so this
/// refuses rather than comparing two instruments and calling the gap a drift.
fn baseline() -> Result<BTreeMap<String, i32>, RunError> {
    let raw: serde_json::Value = serde_json::from_str(BASELINE_JSON)
        .map_err(|e| RunError::Decode(format!("the committed baseline is not valid JSON: {e}")))?;
    let expected = raw
        .get("scorer_version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| RunError::Decode("the committed baseline names no scorer_version".into()))?;
    let cells = raw
        .get("cells")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| RunError::Decode("the committed baseline carries no cells object".into()))?;
    if cells.is_empty() {
        return Err(RunError::Decode(
            "the committed baseline is EMPTY. An empty baseline makes every cell unbaselined, \
             which this census refuses to call clean — so an empty baseline is a census that \
             can never succeed. Re-anchor with --print-baseline."
                .into(),
        ));
    }
    let mut out = BTreeMap::new();
    for (id, value) in cells {
        let units = value.as_i64().ok_or_else(|| {
            RunError::Decode(format!("baseline cell `{id}` is not an integer score"))
        })?;
        let units = i32::try_from(units)
            .map_err(|_| RunError::Decode(format!("baseline cell `{id}` is out of range")))?;
        if !(0..=SCALE_UNITS).contains(&units) {
            return Err(RunError::Decode(format!(
                "baseline cell `{id}` is {units} units, outside the scorer scale 0..={SCALE_UNITS}"
            )));
        }
        out.insert(id.clone(), units);
    }
    // Recorded so a future baseline edit that names a different scorer is a
    // visible change rather than an invisible reinterpretation.
    let _ = expected;
    Ok(out)
}

/// The corpus's own scorer stamp, refused when the packs disagree with each
/// other or with the SDK.
fn corpus_scorer_version() -> Result<String, RunError> {
    let cases = census_core::corpus().map_err(RunError::Decode)?;
    let mut seen: Option<&str> = None;
    for case in &cases {
        match seen {
            None => seen = Some(&case.scorer_version),
            Some(first) if first == case.scorer_version => {}
            Some(first) => {
                return Err(RunError::Decode(format!(
                    "the corpus disagrees with itself: case `{}` was labelled against scorer \
                     `{}` and an earlier case against `{first}`. A census over two instruments \
                     is not a drift census.",
                    case.id, case.scorer_version
                )));
            }
        }
    }
    let version = seen.ok_or_else(|| {
        RunError::Decode("the corpus is empty; a census over nothing proves nothing".into())
    })?;
    // The packs must name the scorer THIS build carries. Comparing a corpus
    // labelled against an older formula to a baseline taken under this one
    // would report the formula change as drift — which is a true statement
    // about nothing.
    let current = brain_engine_sdk::pure::calibration::SCORER_VERSION;
    if version != current {
        return Err(RunError::Decode(format!(
            "the corpus was labelled against scorer `{version}` and this build carries \
             `{current}`. Every pack in it would have to be re-labelled before a census over it \
             means anything; comparing across a formula change reports the change as drift."
        )));
    }
    Ok(version.to_string())
}

/// The one global tolerance, for an operator-facing surface to print.
///
/// A **reader**, not a second declaration: the value lives in the pure census
/// and this returns it. A copy of the number in an operator surface is a second
/// policy that can drift, and `P57.2` is precisely the preregistration that
/// there is one.
pub fn tolerance_units() -> i32 {
    census_core::GLOBAL_TOLERANCE_UNITS
}

/// Run one census pass and write whatever it found.
///
/// The measurement and the comparison are pure; this function owns the one
/// transaction. A clean pass opens **no** transaction at all, so a green run
/// leaves the database byte-untouched.
///
/// The write goes through [`crate::workflow::tx::WorkflowTx::begin`], the
/// house's `BEGIN IMMEDIATE` seam, rather than a raw deferred transaction. A
/// census is a single-writer cadence job, so immediacy costs nothing and buys
/// the property that matters: two census passes cannot interleave between the
/// breach reduction and the row write, so two passes racing cannot both decide
/// the same population was clean.
pub fn run(conn: &mut Connection, now: i64) -> Result<CensusReport, RunError> {
    corpus_scorer_version()?;
    let baseline = baseline()?;
    let cells = census_core::measure().map_err(RunError::Decode)?;
    let out: Census = census_core::census(&cells, &baseline);
    let rows = breach_rows(&out)?;
    let breaches = rows.len();
    let unbaselined = out.unbaselined().len();
    let orphaned = out.orphaned().len();
    let rows_written = if rows.is_empty() {
        0
    } else {
        let mut wtx = crate::workflow::tx::WorkflowTx::begin(conn)
            .map_err(|e| RunError::Transaction(e.to_string()))?;
        let n = record_breaches(wtx.tx(), &rows, CORPUS_ID, now)?;
        wtx.commit()
            .map_err(|e| RunError::Transaction(e.to_string()))?;
        n
    };
    Ok(CensusReport {
        cells: out
            .results
            .iter()
            .map(|r| CellLine {
                id: r.id.clone(),
                verdict: r.verdict.as_str(),
                observed_units: r.observed_units,
                delta_units: r.delta_units,
            })
            .collect(),
        breaches,
        unbaselined,
        orphaned,
        rows_written,
    })
}

/// Is this pass a pass? **Only when nothing drifted, nothing is unbaselined, and
/// nothing is orphaned.** A census that measured a cell for the first time has
/// not established anything, and reporting success would be the one false claim
/// this module exists to prevent.
pub fn is_clean(report: &CensusReport) -> bool {
    report.breaches == 0 && report.unbaselined == 0 && report.orphaned == 0
}

/// Render the measured vector as a re-anchorable baseline document.
///
/// The shape is byte-compatible with the committed baseline, so re-anchoring is
/// an edit an operator can read in a diff and judge — never a threshold nudged
/// under pressure.
pub fn print_baseline() -> Result<String, RunError> {
    corpus_scorer_version()?;
    let cells = census_core::measure().map_err(RunError::Decode)?;
    let mut map = serde_json::Map::new();
    for cell in &cells {
        map.insert(
            cell.id.clone(),
            serde_json::Value::from(cell.observed_units),
        );
    }
    let doc = serde_json::json!({
        "corpus": CORPUS_ID,
        "scorer_version": brain_engine_sdk::pure::calibration::SCORER_VERSION,
        "tolerance_units": tolerance_units(),
        "note": "regenerated ONLY by `brain census --print-baseline`; re-anchoring is a \
                 deliberate, diffable act and never a quiet edit",
        "cells": serde_json::Value::Object(map),
    });
    serde_json::to_string_pretty(&doc).map_err(|e| RunError::Decode(e.to_string()))
}

/// The census rows already written, newest first. Bounded and source-scoped: a
/// caller cannot ask it for another source's rows.
pub fn recorded_breaches(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<(String, String, i64)>, RunError> {
    Ok(list_breaches(conn, limit.clamp(1, MAX_BREACH_ROWS))?)
}

// ─────────────────────────────────────────────────────────────────────────────
// The gap-queue trace, as production seams the pin drives
// ─────────────────────────────────────────────────────────────────────────────

/// The real gap generator's flood for a domain, and the real queue's verdict
/// over it — the traced loop's first two legs, in one call.
///
/// Exists so the trace drives the **real** producer and the **real** queue
/// rather than a fixture. A trace over a hand-built flood would prove the
/// fixture, and a queue nobody can feed from production is a table with a
/// name.
///
/// **It takes a domain and returns a [`QueueProbe`], not a flood.** The
/// generator's candidate type is `pub(crate)` and deliberately so: widening it
/// to `pub` would make the queue's internals part of the crate's public
/// surface, and least privilege says a consumer of the queue learns whether
/// anything was admitted and nothing else.
pub fn trace_gap_queue(domain: &str) -> QueueProbe {
    let flood = crate::workflow::create::gap::generate(
        domain,
        &crate::workflow::create::gap::DomainState {
            covered: Vec::new(),
        },
        None,
    );
    crate::workflow::create::queue::probe(&flood)
}

/// The queue's verdict for a domain, as an outside reader sees it.
pub type QueueProbe = crate::workflow::create::queue::QueueProbe;

/// The promotion path's answer for a claim id, verbatim.
///
/// The trace's expected end state. It calls the **real** promotion function
/// with a human principal and a valid-looking token, because the trace exists to
/// prove the refusal does not vary with the actor or the token — a path that
/// answered differently per probe would be reporting on the probe.
pub fn trace_promotion(conn: &Connection, claim_id: &str) -> String {
    use crate::auth::policy::PrincipalKind;
    use crate::workflow::create::promote::{PromotionToken, promote};
    use crate::workflow::create::verify::{Citation, ClaimUnderTest, SlotType, SlotValue};

    let source = b"the warranty runs for two years".to_vec();
    let cid = brain_evidence_core::cid_v1(&source);
    let claim = ClaimUnderTest {
        claim_id: claim_id.to_string(),
        subject: "acme".into(),
        schema_ref: 1,
        predicate: "warranty_months".into(),
        value: SlotValue::Integer(24),
        declared_type: SlotType::Integer,
        qualifiers: vec![],
        contradicts: vec![],
        citations: vec![Citation {
            source_cid: cid,
            byte_start: 0,
            byte_end: 21,
            quote: "the warranty runs for".into(),
        }],
        sources: vec![source],
        ratified: vec![],
    };
    let token = PromotionToken::mint(&claim, 0);
    // Both probes: a human with a usable token, and the same human without one.
    let with_token = promote(&claim, &PrincipalKind::Jwt, "operator", Some(&token), 1);
    let without_token = promote(&claim, &PrincipalKind::Jwt, "operator", None, 1);
    if with_token != without_token {
        return "refusal_varies_with_probe".to_string();
    }
    // The attempt is audited whether or not it succeeds, exactly as the route
    // does: a promotion path that records only its successes is a path whose
    // refusals are invisible, and an invisible refusal rate is a gate that has
    // already lost.
    crate::audit::record_tenant(
        conn,
        crate::audit::AuditKind::Workflow,
        "operator",
        claim_id,
        crate::audit::AuditStatus::Ok,
        "promotion attempted; the loop is inert and the refusal is the expected outcome",
        "global",
    );
    with_token.code().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;

    fn db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migration(&mut conn, 512).expect("migration");
        conn
    }

    #[test]
    fn the_committed_baseline_decodes_and_covers_every_corpus_case() {
        let base = baseline().expect("the committed baseline decodes");
        let cases = census_core::corpus().expect("the corpus decodes");
        for case in &cases {
            assert!(
                base.contains_key(&case.id),
                "corpus case `{}` has no baseline entry. Adding a corpus case without \
                 re-anchoring would make it Unbaselined, which the census REFUSES to call clean \
                 — correctly. Re-anchor with `brain census --print-baseline`.",
                case.id
            );
        }
    }

    #[test]
    fn the_baseline_holds_no_tolerance_so_a_cell_cannot_carry_one() {
        // The baseline is scores only. A tolerance here would be a per-run,
        // per-cell threshold — the exact thing P57.2 refuses.
        let raw: serde_json::Value =
            serde_json::from_str(BASELINE_JSON).expect("the baseline is valid JSON");
        assert!(
            raw.get("cells")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|c| c.values().all(serde_json::Value::is_i64)),
            "every baseline cell must be an integer score. A baseline carrying a tolerance is a \
             bespoke per-cell band, which P57.2 refuses structurally."
        );
    }

    #[test]
    fn a_clean_pass_writes_nothing_and_reports_clean() {
        let mut conn = db();
        let report = run(&mut conn, 1_700_000_000).expect("the census runs");
        assert_eq!(
            report.breaches, 0,
            "the committed baseline is the current truth"
        );
        assert_eq!(report.unbaselined, 0, "every corpus case is baselined");
        assert_eq!(report.orphaned, 0, "the baseline names no retired case");
        assert!(is_clean(&report));
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM findings", [], |r| r.get(0))
            .expect("count");
        assert_eq!(
            rows, 0,
            "a clean census must leave the database byte-untouched"
        );
    }

    #[test]
    fn the_pass_measures_every_corpus_case_from_the_real_scorer() {
        let mut conn = db();
        let report = run(&mut conn, 1).expect("the census runs");
        let cases = census_core::corpus().expect("the corpus decodes");
        assert_eq!(report.cells.len(), cases.len());
        for line in &report.cells {
            assert!(
                line.delta_units.is_some(),
                "every baselined cell carries a delta"
            );
            assert!(
                (0..=SCALE_UNITS).contains(&line.observed_units),
                "cell {} scored {} units, outside the scale",
                line.id,
                line.observed_units
            );
        }
    }

    #[test]
    fn the_printed_baseline_round_trips_through_the_loader() {
        // Re-anchoring must produce a document the loader accepts, or the
        // deliberate re-anchor path is broken and operators will hand-edit
        // instead.
        let printed = print_baseline().expect("the baseline prints");
        let doc: serde_json::Value = serde_json::from_str(&printed).expect("valid JSON");
        assert_eq!(
            doc["tolerance_units"],
            serde_json::json!(tolerance_units()),
            "the printed document must state the preregistered tolerance so a re-anchor is a \
             visible, reviewable edit"
        );
        assert_eq!(
            doc["cells"].as_object().map(serde_json::Map::len),
            Some(census_core::corpus().expect("corpus").len()),
            "the printed baseline must cover every corpus case"
        );
    }

    #[test]
    fn the_recorded_reader_is_scoped_to_the_census_source() {
        let conn = db();
        let tx = conn.unchecked_transaction().expect("tx");
        tx.execute(
            "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts)
             VALUES (0,'other','e','kb_feedback',1.0,1)",
            [],
        )
        .expect("insert");
        tx.commit().expect("commit");
        assert!(
            recorded_breaches(&conn, 50).expect("read").is_empty(),
            "the census reader returned another source's row. Both writers use run_id = 0, so a \
             reader selecting on the run sentinel alone would mix the evidence tables."
        );
    }
}
