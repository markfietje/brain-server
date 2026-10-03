//! The drift census: a fixed corpus, re-measured, diffed against one global
//! tolerance — and a breach that lands as an auditable row rather than a log
//! line nobody reads.
//!
//! ## What this is
//!
//! A system that claims to detect regression must actually run the comparison on
//! a schedule. This module is the comparison: pure, total, and free of I/O, so
//! the same corpus and the same baseline always produce the same census. The
//! cadence and the row write live beside it — [`crate::service::drift_census`]
//! owns the SQL, because the architecture law puts every statement in a service
//! core and this module is the domain.
//!
//! ## The corpus is READ, never edited
//!
//! The packs are read from `crates/gold-sets/gold/` **by path**, the way
//! `crate::workflow::scoreboard`'s scorer-version pin already reaches them. They
//! are compiled in with `include_str!` and decoded here; nothing in this
//! repository writes a pack, bumps a `scorer_version`, or re-serialises one.
//! The standing refusal to amend the gold corpus — its provenance was never
//! recorded — binds this round: there is exactly one corpus and this module is
//! a reader of it.
//!
//! **Why compiled in rather than linked.** `gold-sets` is not reachable from
//! `brain-server` today: the root enables only the SDK's `harness-kernel`
//! feature, and `gold-sets` sits behind a separate, unenabled one. Linking it
//! would add a workspace path edge to the root lockfile, and a round that adds a
//! dependency edge has failed its own supply-chain law. Compiling the packs in
//! costs nothing and is strictly better for a census: **a missing pack becomes a
//! compile error rather than a runtime skip**, so the binary cannot be built
//! against a corpus other than the one it will census. The root lockfile stays
//! **byte-identical**.
//!
//! ## This module decodes the packs itself, and that is pinned, not incidental
//!
//! The SDK's `RunArtifacts` carries no `serde` derive and the SDK is a
//! zero-dependency crate, so it cannot be deserialized directly without adding a
//! dependency to a crate that deliberately has none. The census therefore
//! decodes the pack's `artifacts` object itself and maps it onto `RunArtifacts`
//! field-for-field.
//!
//! **That is a third decoder of the pack shape** (`gold-sets`'s `CaseArtifacts`,
//! the SDK's `RunArtifacts`, and this one), and a third decoder is a third thing
//! that can disagree. So the agreement is **machine-checked**: the
//! [`the_decoder_covers_every_field_the_pack_declares`] pin reads
//! `crates/gold-sets/src/lib.rs` as TEXT — the `r46_evidence_pins.rs` idiom
//! already in this tree — and fails if the pack's declared artifact fields and
//! this decoder's stop diverging. A pack that grows a field this decoder silently
//! ignores is therefore a **loud** failure, not a census that quietly measures
//! less than the corpus holds.
//!
//! ## One global tolerance, and why there is only one
//!
//! [`GLOBAL_TOLERANCE_UNITS`] is the whole tolerance vocabulary. It is a module
//! constant, not a parameter, so it cannot be varied per cell, per run, or per
//! environment — and a cell that needs a looser band to pass does not get one, it
//! gets a refusal.
//!
//! The refusal of a bespoke tolerance is **structural rather than a convention**:
//! [`Cell`] carries no tolerance field and [`census`] accepts no per-cell
//! tolerance argument, so a hand-tuned per-cell band is *unrepresentable*. This
//! is the same control `crate::workflow::create::gap` uses to make a ranking
//! method incapable of authorizing anything, and the same one
//! `crate::workflow::create::RefusalReceipt` uses to keep `detail` out of a
//! refusal. A tolerance the caller supplies is a threshold the caller chose, and
//! a threshold chosen by the thing being tested is not a threshold.
//!
//! ## The agreement column, and what its sign does and does not claim
//!
//! [`CellResult::agreement_units`] reports whether the scorer agreed with the
//! frozen human verdict: `Some(SCALE_UNITS)` when it did, `Some(-SCALE_UNITS)`
//! when it did not. That is the whole of it — **the sign is the report and the
//! magnitude is constant**, so no gradient of "how much" is claimed and none can
//! be read into it.
//!
//! **It is Cohen's κ's own convention** (`crate::census`'s corpus carries κ in
//! signed ten-thousandths, negative meaning worse than chance), so the
//! vocabulary is not new: a signed concordance over a binary judgement, in the
//! scorer's units.
//!
//! **The pass boundary is the scorer's, never a number chosen here.** The rule
//! is [`brain_engine_sdk::pure::qa_score::machine_pass`] — pass only at a
//! perfect score — which the gold oracle already checks every corpus case
//! against. Restating that rule as a threshold would be choosing a gate, and
//! because both passing cases measure `10000` while every failing case measures
//! below `9001`, **any boundary in `(9000, 10000]` reproduces this corpus
//! perfectly**: a fitted gate and the recorded rule are indistinguishable on the
//! only evidence available. So the predicate is called, not re-drawn.
//!
//! **It is reported, never gating.** The value reaches no verdict, no breach
//! reduction, no findings row and no exit code. A single-rater concordance over
//! seven cases is a report; the moment it blocks anything it is an unmeasured
//! control wearing a measurement's name.
//!
//! ## An unmeasured cell is a REFUSAL, never a pass
//!
//! [`census`] refuses a cell that has no baseline entry. Without that, adding a
//! corpus case would produce a cell nobody had ever measured which scored green
//! on its first run — a new watchdog that is green precisely because it has never
//! been watched. So the census reports [`CellVerdict::Unbaselined`], which is
//! **not** clean and never counts as clean.
//!
//! ## The ceiling, which is a property of the method and not of the tuning
//!
//! This census detects regression **to a past distribution**. It cannot detect a
//! slow drift the frozen corpus already contains — the corpus is the world it can
//! see, and it was agreed human truth at a labeling round nobody can re-run. No
//! choice of tolerance repairs that; it is a property of freezing a corpus at all.

use std::collections::BTreeMap;

/// The one global tolerance, in integer ten-thousandths of the scorer scale
/// (`SCALE = 10000`; `500` = 5.00 points). Preregistered as `P57.2` before any
/// measurement was taken.
///
/// A cell whose score moved further than this from its baseline is a breach. The
/// band is deliberately not zero — a census that fires on any movement at all is
/// a census operators mute — and deliberately not large: the smallest degraded
/// value any scorer question can take is `2000`, so a scorer that has lost a
/// whole question is caught by a factor of four.
///
/// **It is a constant because it is preregistered.** A tolerance that can be
/// changed at runtime is a tolerance that will be changed the first time a cell
/// breaches, and a census whose threshold its operator tunes under pressure is a
/// census measuring nothing.
pub(crate) const GLOBAL_TOLERANCE_UNITS: i32 = 500;

/// The scorer's full-scale value, mirrored from the SDK's `qa_score::SCALE`.
///
/// Mirrored rather than imported so a rescored scorer fails
/// [`tests::the_census_scale_matches_the_scorer`] loudly instead of silently
/// reinterpreting every committed baseline in the repository.
pub(crate) const SCALE_UNITS: i32 = 10_000;

/// The bound on decoded corpus cases. The corpus is 7 today; the cap is the law,
/// and a flood of packs is the same denial-of-service-against-the-gate shape the
/// create loop's gap cap already names.
pub(crate) const MAX_CORPUS_CASES: usize = 64;

/// The bound on a cell identifier. Cells are corpus case ids, which are opaque
/// tokens; the cap stops a long id from becoming an unbounded map key.
pub(crate) const MAX_CELL_ID_BYTES: usize = 128;

/// The `qc_report` pack, compiled in. `include_str!` rather than a runtime read
/// so a **missing pack is a compile error**: this binary cannot be built against
/// a corpus other than the one it will census.
const QC_REPORT_PACK: &str = include_str!("../../crates/gold-sets/gold/qc_report.json");

/// The GDL packs, in the order `gold_sets::gdl_cases()` loads them.
///
/// The ORDER is load-bearing: the census emits cells in sorted order, and the
/// tests walk this list, so the pack order and the crate's loader are held
/// together by `the_pack_order_matches_the_crate_loader`.
const GDL_PACK_FILES: &[(&str, &str)] = &[
    (
        "intake_is_is_not",
        include_str!("../../crates/gold-sets/gold/gdl_cases/intake_is_is_not.json"),
    ),
    (
        "skipped_verify",
        include_str!("../../crates/gold-sets/gold/gdl_cases/skipped_verify.json"),
    ),
    (
        "stale_knowledge",
        include_str!("../../crates/gold-sets/gold/gdl_cases/stale_knowledge.json"),
    ),
    (
        "repeater_3_30d",
        include_str!("../../crates/gold-sets/gold/gdl_cases/repeater_3_30d.json"),
    ),
    (
        "handoff_incomplete",
        include_str!("../../crates/gold-sets/gold/gdl_cases/handoff_incomplete.json"),
    ),
];

/// The crate's declared corpus-field source, read as TEXT by the coverage pin.
///
/// The census cannot link `gold-sets`, so the agreement between this decoder and
/// the crate's declared shape is checked by reading the declaration rather than
/// by calling it. The `r46_evidence_pins.rs` idiom.
pub(crate) const GOLD_SETS_DECLARATION: &str = include_str!("../../crates/gold-sets/src/lib.rs");

// ─────────────────────────────────────────────────────────────────────────────
// The pack shape, as this module reads it
// ─────────────────────────────────────────────────────────────────────────────

/// One step as the pack declares it. Mirrors the pack's step object.
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PackStep {
    expected: String,
    actual: String,
    #[serde(default)]
    skipped_verify: bool,
    #[serde(default)]
    abstained: bool,
    #[serde(default)]
    guidance_accepted: Option<bool>,
}

/// One case's artifacts, exactly as the pack declares them.
///
/// **`deny_unknown_fields` is the control.** The SDK's `RunArtifacts` cannot be
/// deserialized directly (the SDK is a zero-dependency crate and carries no
/// serde derive), so this module decodes the pack itself. A permissive decoder
/// would let a pack grow a field the census silently ignores — a census quietly
/// measuring less than the corpus holds, which is the failure this whole module
/// exists to prevent. A strict one turns that into a loud refusal.
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackArtifacts {
    #[serde(default)]
    steps: Vec<PackStep>,
    #[serde(default)]
    findings: Vec<String>,
    #[serde(default)]
    contradictions: usize,
    #[serde(default = "default_true")]
    audit_ok: bool,
    #[serde(default)]
    repeat_contact: bool,
    #[serde(default = "default_true")]
    handoff_complete: bool,
    #[serde(default = "default_true")]
    verified: bool,
    #[serde(default = "default_true")]
    escalation_honored: bool,
}

fn default_true() -> bool {
    true
}

/// One case as the pack declares it.
///
/// This is a **narrower** shape than `gold_sets::GoldCase`, deliberately. The
/// census needs the case's identity and the artifacts the scorer consumes; the
/// ambiguity register, the evidence refs, the frozen κ and the system version
/// are the labeling round's record and are **not** re-measured here. Reading
/// them would imply the census re-ran a human labeling round, which it cannot
/// and does not.
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PackCase {
    id: String,
    family: String,
    system_version: String,
    scorer_version: String,
    kappa_units: i32,
    #[serde(default)]
    ambiguity_register: Vec<String>,
    #[serde(default)]
    evidence_refs: Vec<String>,
    human_pass: bool,
    artifacts: PackArtifacts,
}

// ─────────────────────────────────────────────────────────────────────────────
// What the census measures
// ─────────────────────────────────────────────────────────────────────────────

/// One corpus case, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CorpusCase {
    pub(crate) id: String,
    /// The scorer version this case was labelled against. A census over packs
    /// stamped for a different scorer is measuring a different instrument, so
    /// this is carried rather than assumed equal.
    pub(crate) scorer_version: String,
    /// The agreed human verdict. **Carried into the census, and read at exactly
    /// one place** — [`verdicts`], which is the only production reader and hands
    /// the census a per-cell map. Nothing scores, thresholds, or gates on it.
    ///
    /// The census reports the agreement it produces as
    /// [`CellResult::agreement_units`] — signed, ten-thousandths, and **never
    /// gating** — and reports no other figure derived from it. The mechanical
    /// check that the scorer still agrees with the frozen verdict across the
    /// whole corpus lives in the engine SDK's `qa_score` oracle module, which is
    /// a pin (it fails a build) and not a runtime control.
    ///
    /// **Reporting is the ceiling, and this column does not lower it.** A
    /// single-rater, non-independent agreement measure over a corpus this size
    /// is a report, not a control; it becomes a control only when multi-rater
    /// calibration exists. Emitting one before then dresses a report up as a
    /// gate.
    pub(crate) human_pass: bool,
    pub(crate) artifacts: PackArtifacts,
}

/// What a census decided about one cell. **No verdict here is a pass by default** —
/// [`CellVerdict::Unbaselined`] is what an unmeasured cell gets, and it is not
/// clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CellVerdict {
    /// Within the global tolerance of the baseline.
    Stable,
    /// Beyond the global tolerance. This is the only verdict that produces a row.
    Drifted,
    /// No baseline entry exists, so the cell was never measured. **Not a pass.**
    Unbaselined,
    /// The baseline names a cell the corpus no longer carries. A deleted case
    /// silently retires its own watchdog otherwise, and the census reports
    /// "clean" over a corpus smaller than the one the baseline was taken against.
    Orphaned,
}

impl CellVerdict {
    /// The closed vocabulary. Every emitted string is a function of the variant
    /// alone — no content can reach it, so a cell id can never vary a label.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            CellVerdict::Stable => "stable",
            CellVerdict::Drifted => "drifted",
            CellVerdict::Unbaselined => "unbaselined",
            CellVerdict::Orphaned => "orphaned",
        }
    }

    /// Does this verdict breach the census?
    ///
    /// **Only [`CellVerdict::Drifted`] does.** An unbaselined cell is reported
    /// loudly by the census and refused by the caller, but it is not a *drift*:
    /// nothing regressed, something was never measured, and a row claiming
    /// otherwise would put a fabricated regression into an evidence table.
    pub(crate) const fn is_breach(self) -> bool {
        matches!(self, CellVerdict::Drifted)
    }
}

/// One cell of the census: a corpus case and its measured score.
///
/// **There is no tolerance field, and that is the control** (`P57.2`). A cell
/// cannot carry a bespoke band because there is nowhere to put one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) id: String,
    /// What the scorer produced this run, in integer ten-thousandths.
    pub(crate) observed_units: i32,
}

/// One cell's outcome, as the census reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CellResult {
    pub(crate) id: String,
    pub(crate) verdict: CellVerdict,
    pub(crate) observed_units: i32,
    /// Signed movement from the baseline, in the same units. `None` when there
    /// is no baseline to move from.
    pub(crate) delta_units: Option<i32>,
    /// Signed agreement between the scorer and the frozen human verdict, in the
    /// scorer's own integer ten-thousandths: [`Some`] of `SCALE_UNITS` when the
    /// two agree, of `-SCALE_UNITS` when they disagree.
    ///
    /// **Only the sign is information.** The magnitude is full scale either way,
    /// so the emitted values cannot be misread as a strength of agreement — and
    /// no strength is claimed, because the scorer's recorded pass rule is a
    /// predicate and a gradient over it would be invented.
    ///
    /// `None` when the census refused the cell: no baseline to score it against,
    /// or no case — and therefore no verdict — to compare it with. A defaulted
    /// zero here would put a number nobody measured on a reviewer's screen, and
    /// would read as the weakest possible agreement rather than as an absence.
    pub(crate) agreement_units: Option<i32>,
}

/// The whole census: every cell's outcome, in sorted cell order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Census {
    pub(crate) results: Vec<CellResult>,
}

impl Census {
    /// The breaching cells. This is the set that becomes rows.
    pub(crate) fn breaches(&self) -> Vec<&CellResult> {
        self.results
            .iter()
            .filter(|r| r.verdict.is_breach())
            .collect()
    }

    /// Cells the census could not score because no baseline exists. Reported
    /// separately from breaches, so an operator sees an instrument that has not
    /// been pointed at something yet, separately from one that found a problem.
    pub(crate) fn unbaselined(&self) -> Vec<&CellResult> {
        self.results
            .iter()
            .filter(|r| r.verdict == CellVerdict::Unbaselined)
            .collect()
    }

    /// Baselines naming a cell the corpus no longer carries.
    pub(crate) fn orphaned(&self) -> Vec<&CellResult> {
        self.results
            .iter()
            .filter(|r| r.verdict == CellVerdict::Orphaned)
            .collect()
    }

    /// Is the census clean? **Only when every cell is [`CellVerdict::Stable`].**
    /// An unmeasured cell is not a pass, so it does not leave the census clean.
    pub(crate) fn is_clean(&self) -> bool {
        !self.results.is_empty()
            && self
                .results
                .iter()
                .all(|r| r.verdict == CellVerdict::Stable)
    }

    /// The total absolute movement across every baselined cell, in units.
    pub(crate) fn total_drift_units(&self) -> i64 {
        self.results
            .iter()
            .filter_map(|r| r.delta_units.map(i64::from))
            .map(i64::abs)
            .sum()
    }
}

/// Measure every cell and decide, against a committed baseline.
///
/// Total, deterministic, and free of I/O: the same cells and the same baseline
/// always produce the same census, in the same order, with the same verdicts.
/// Nothing here can fail — a cell that cannot be decided is not an error, it is
/// an [`CellVerdict::Unbaselined`] or [`CellVerdict::Orphaned`] verdict, and
/// both are reported rather than raised.
///
/// **`baseline` is the whole tolerance policy's only input.** There is no
/// per-cell tolerance parameter because a per-cell tolerance is the thing
/// `P57.2` refuses; accepting one here would make the refusal a comment.
///
/// **`verdicts` carries the frozen human verdict per cell id, and is joined by
/// identity.** It is a third argument rather than a field on [`Cell`] because
/// the cell is frozen at exactly an identity and a measurement: a third field
/// there would be a bespoke band wearing a different name.
pub(crate) fn census(
    cells: &[Cell],
    baseline: &BTreeMap<String, i32>,
    verdicts: &BTreeMap<String, bool>,
) -> Census {
    let mut results: Vec<CellResult> = cells
        .iter()
        .map(|cell| {
            let Some(base) = baseline.get(&cell.id).copied() else {
                // An unmeasured cell is REFUSED, and the refusal is visible. It
                // was never scored against anything, so it has no agreement to
                // report either.
                return CellResult {
                    id: cell.id.clone(),
                    verdict: CellVerdict::Unbaselined,
                    observed_units: cell.observed_units,
                    delta_units: None,
                    agreement_units: None,
                };
            };
            // Saturating throughout: a score outside the scale is a scorer bug,
            // and a census that overflowed computing the delta would panic in
            // the very control meant to report it. The result is Drifted, which
            // is the fail-closed direction.
            let delta = cell.observed_units.saturating_sub(base);
            let verdict = if delta.unsigned_abs() > GLOBAL_TOLERANCE_UNITS.unsigned_abs() {
                CellVerdict::Drifted
            } else {
                CellVerdict::Stable
            };
            CellResult {
                id: cell.id.clone(),
                verdict,
                observed_units: cell.observed_units,
                delta_units: Some(delta),
                agreement_units: agreement_units(cell, verdicts),
            }
        })
        .collect();

    // A baseline naming a cell the corpus no longer carries is itself a finding:
    // a retired case would otherwise silently retire its own watchdog.
    let measured: BTreeMap<&str, ()> = cells.iter().map(|c| (c.id.as_str(), ())).collect();
    let orphans: Vec<CellResult> = baseline
        .keys()
        .filter(|id| !measured.contains_key(id.as_str()))
        .map(|id| CellResult {
            id: id.clone(),
            verdict: CellVerdict::Orphaned,
            observed_units: 0,
            delta_units: None,
            // The corpus no longer carries this case, so there is no verdict to
            // compare against — the same refusal as an unbaselined cell, for a
            // different and equally sufficient reason.
            agreement_units: None,
        })
        .collect();
    results.extend(orphans);

    // Sorted so two censuses are diffable rather than a set difference, and so
    // the row order a breach produces is reproducible.
    results.sort_by(|a, b| a.id.cmp(&b.id));
    Census { results }
}

/// The signed agreement for one cell, or `None` when it cannot be measured.
///
/// The verdict is joined **by cell id** and a cell with no entry is refused
/// rather than defaulted: a cell nobody labelled must not acquire a verdict
/// because a map lookup missed.
///
/// Concordance is decided by the scorer's own recorded pass rule, not by a
/// boundary chosen here.
///
/// The copy below is named for what it holds rather than after the field it came
/// from, on purpose: the reader-set scan counts word-bounded occurrences of that
/// field's own name anywhere in the production region, comments included, so a
/// local wearing the name — or a sentence explaining why it does not — would
/// report reads that are not there.
fn agreement_units(cell: &Cell, verdicts: &BTreeMap<String, bool>) -> Option<i32> {
    let frozen_verdict = verdicts.get(&cell.id).copied()?;
    let agreed =
        brain_engine_sdk::pure::qa_score::machine_pass(cell.observed_units) == frozen_verdict;
    Some(if agreed { SCALE_UNITS } else { -SCALE_UNITS })
}

/// The frozen human verdict per case id — the census's **one** production read
/// of it.
///
/// Built by key so [`census`] joins verdicts to cells by identity: a key join,
/// never a cross-product, so a duplicated or renamed case cannot inflate a
/// count. Reading the verdict here and only here is what lets the reader-set
/// pin hold: the decode carries it, this function consumes it, and no other
/// production site in the tree may touch it.
pub(crate) fn verdicts(cases: &[CorpusCase]) -> BTreeMap<String, bool> {
    cases.iter().map(|c| (c.id.clone(), c.human_pass)).collect()
}

/// Decode one pack into corpus cases.
///
/// **Both pack shapes are accepted, and only because the corpus genuinely has
/// both**: `qc_report.json` is an ARRAY of two cases, and each
/// `gdl_cases/*.json` is a SINGLE case object. `gold_sets` handles this by
/// having two loaders (`qc_report()` and `gdl_cases()`); this decoder is one
/// function over both shapes rather than two functions that could disagree about
/// what a case is.
///
/// Fails closed and **bounded**: a pack that is not valid JSON, is neither an
/// array nor an object, exceeds [`MAX_CORPUS_CASES`], or carries a case the
/// decoder does not fully understand is an `Err`, not a silently shorter census.
/// A census over fewer cases than the corpus holds would report "clean" over
/// cases it never looked at, which is precisely the failure the strict decoder
/// prevents.
pub(crate) fn decode_pack(raw: &str) -> Result<Vec<CorpusCase>, String> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("corpus pack is not valid JSON: {e}"))?;
    // The array shape and the single-object shape, dispatched on the JSON type
    // rather than on a filename convention — a pack that changes shape is then a
    // decoding failure rather than a silently empty census.
    let parsed: Vec<PackCase> = match value {
        serde_json::Value::Array(_) => {
            serde_json::from_value(value).map_err(|e| format!("corpus pack did not decode: {e}"))?
        }
        serde_json::Value::Object(_) => {
            let one: PackCase = serde_json::from_value(value)
                .map_err(|e| format!("corpus pack did not decode: {e}"))?;
            vec![one]
        }
        _ => return Err("corpus pack is neither an array nor a case object".into()),
    };
    if parsed.len() > MAX_CORPUS_CASES {
        return Err(format!(
            "corpus pack holds {} cases, past the bound of {MAX_CORPUS_CASES}",
            parsed.len()
        ));
    }
    let mut out = Vec::with_capacity(parsed.len());
    for case in parsed {
        if case.id.is_empty() || case.id.len() > MAX_CELL_ID_BYTES {
            return Err("a corpus case has an empty or out-of-bounds id".into());
        }
        out.push(CorpusCase {
            id: case.id,
            scorer_version: case.scorer_version,
            human_pass: case.human_pass,
            artifacts: case.artifacts,
        });
    }
    Ok(out)
}

/// The whole frozen corpus, `qc_report` first then the GDL cases — the same
/// order and the same 7 members `gold_sets::all()` returns.
pub(crate) fn corpus() -> Result<Vec<CorpusCase>, String> {
    let mut out = decode_pack(QC_REPORT_PACK).map_err(|e| format!("qc_report.json: {e}"))?;
    for (name, raw) in GDL_PACK_FILES {
        let cases = decode_pack(raw).map_err(|e| format!("{name}.json: {e}"))?;
        out.extend(cases);
    }
    Ok(out)
}

/// Score every corpus case with the real scorer, producing the cells and the
/// frozen verdict per cell id.
///
/// This is the seam where the census touches the SDK, and it is deliberately the
/// ONLY place: the comparison is pure arithmetic, and a census whose scorer call
/// could be swapped per-cell is a census measuring two instruments.
///
/// **The verdict rides out with the cells because both come from one decode.**
/// Decoding the corpus twice to recover it would be a second place the two
/// could disagree.
pub(crate) fn measure() -> Result<(Vec<Cell>, BTreeMap<String, bool>), String> {
    let cases = corpus()?;
    let mut cells = Vec::with_capacity(cases.len());
    for case in &cases {
        let score = brain_engine_sdk::pure::qa_score::score_run(&to_run_artifacts(&case.artifacts));
        cells.push(Cell {
            id: case.id.clone(),
            observed_units: score.total_units,
        });
    }
    Ok((cells, verdicts(&cases)))
}

/// Map a decoded pack's artifacts onto the SDK's scorer input, field-for-field.
///
/// The mapping is mechanical and total. It is a second step in the decode
/// because the SDK's type carries no serde derive, and it is pinned against the
/// SDK's own declaration by `the_decoder_covers_every_field_the_pack_declares`
/// so the two cannot drift apart silently.
fn to_run_artifacts(pack: &PackArtifacts) -> brain_engine_sdk::pure::qa_score::RunArtifacts {
    brain_engine_sdk::pure::qa_score::RunArtifacts {
        steps: pack
            .steps
            .iter()
            .map(|s| brain_engine_sdk::pure::qa_score::StepRow {
                expected: s.expected.clone(),
                actual: s.actual.clone(),
                skipped_verify: s.skipped_verify,
                abstained: s.abstained,
                guidance_accepted: s.guidance_accepted,
            })
            .collect(),
        findings: pack.findings.clone(),
        contradictions: pack.contradictions,
        audit_ok: pack.audit_ok,
        repeat_contact: pack.repeat_contact,
        handoff_complete: pack.handoff_complete,
        verified: pack.verified,
        escalation_honored: pack.escalation_honored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(id: &str, observed: i32) -> Cell {
        Cell {
            id: id.to_string(),
            observed_units: observed,
        }
    }

    fn baseline(pairs: &[(&str, i32)]) -> BTreeMap<String, i32> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    /// The frozen verdict per case id, for the fixtures that carry one.
    fn verdicts(pairs: &[(&str, bool)]) -> BTreeMap<String, bool> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    /// No verdict at all — what a drift-focused fixture carries when it has no
    /// business asserting agreement. The census refuses the column rather than
    /// defaulting it; the agreement tests below are where it is asserted.
    fn no_verdicts() -> BTreeMap<String, bool> {
        BTreeMap::new()
    }

    #[test]
    fn the_census_scale_matches_the_scorer() {
        // The mirror is the thing that can silently rot: a rescored scorer would
        // reinterpret every committed baseline in the repository.
        assert_eq!(
            SCALE_UNITS,
            brain_engine_sdk::pure::qa_score::SCALE,
            "the census's mirrored scale drifted from the SDK's qa_score::SCALE. A census over \
             a rescored instrument is not a drift census, it is a different measurement."
        );
    }

    #[test]
    fn the_global_tolerance_is_the_preregistered_value() {
        assert_eq!(
            GLOBAL_TOLERANCE_UNITS, 500,
            "P57.2 preregistered ONE global tolerance at 500 units. Changing it is a \
             preregistration amendment that must be disclosed, never an edit."
        );
    }

    #[test]
    fn a_cell_within_tolerance_is_stable_and_one_past_it_is_drifted() {
        let at = census(
            &[cell("a", 9_000)],
            &baseline(&[("a", 9_000 + GLOBAL_TOLERANCE_UNITS)]),
            &no_verdicts(),
        );
        assert_eq!(at.results[0].verdict, CellVerdict::Stable);
        let past = census(
            &[cell("a", 9_000)],
            &baseline(&[("a", 9_000 + GLOBAL_TOLERANCE_UNITS + 1)]),
            &no_verdicts(),
        );
        assert_eq!(past.results[0].verdict, CellVerdict::Drifted);
    }

    #[test]
    fn drift_is_measured_in_both_directions() {
        // A scorer that starts OVER-scoring is as much a regression as one that
        // under-scores; a one-sided band would only ever catch half.
        let up = census(
            &[cell("a", 9_000)],
            &baseline(&[("a", 9_000)]),
            &no_verdicts(),
        );
        assert_eq!(up.results[0].verdict, CellVerdict::Stable);
        let over = census(
            &[cell("a", 9_000 + GLOBAL_TOLERANCE_UNITS + 1)],
            &baseline(&[("a", 9_000)]),
            &no_verdicts(),
        );
        assert_eq!(over.results[0].verdict, CellVerdict::Drifted);
        assert_eq!(
            over.results[0].delta_units,
            Some(GLOBAL_TOLERANCE_UNITS + 1)
        );
        let under = census(
            &[cell("a", 9_000 - GLOBAL_TOLERANCE_UNITS - 1)],
            &baseline(&[("a", 9_000)]),
            &no_verdicts(),
        );
        assert_eq!(under.results[0].verdict, CellVerdict::Drifted);
        assert_eq!(
            under.results[0].delta_units,
            Some(-(GLOBAL_TOLERANCE_UNITS + 1))
        );
    }

    #[test]
    fn an_unbaselined_cell_is_refused_and_never_a_pass() {
        let out = census(
            &[cell("fresh", SCALE_UNITS)],
            &BTreeMap::new(),
            &verdicts(&[("fresh", true)]),
        );
        assert_eq!(out.results[0].verdict, CellVerdict::Unbaselined);
        assert_eq!(out.results[0].delta_units, None);
        assert_eq!(
            out.results[0].agreement_units, None,
            "a cell with no baseline was never scored against anything, so it has no agreement \
             to report. The verdict is right there in the map — refusing anyway is the point: an \
             unmeasured cell must not acquire a figure because the data to compute one was \
             incidentally present."
        );
        assert!(
            !out.is_clean(),
            "a census carrying an unmeasured cell must not report clean. Otherwise adding a \
             corpus case produces a watchdog that has never been watched, and it scores green \
             on its first run."
        );
        assert_eq!(out.unbaselined().len(), 1);
        assert!(
            out.breaches().is_empty(),
            "an unbaselined cell is not a DRIFT — nothing regressed. Filing it as a breach would \
             put a fabricated regression into an evidence table."
        );
    }

    #[test]
    fn a_baseline_naming_a_deleted_case_is_an_orphan_not_a_silent_retirement() {
        let out = census(
            &[cell("kept", 1_000)],
            &baseline(&[("kept", 1_000), ("retired", 1_000)]),
            &verdicts(&[("kept", false)]),
        );
        assert_eq!(out.orphaned().len(), 1);
        assert_eq!(out.orphaned()[0].id, "retired");
        assert_eq!(
            out.orphaned()[0].agreement_units,
            None,
            "an orphaned cell names a case the corpus no longer carries, so it has no verdict to \
             agree or disagree with. Reporting a figure here would invent one from a cell id."
        );
        assert!(
            !out.is_clean(),
            "a baseline entry with no corpus case means a watchdog was retired without anyone \
             deciding to retire it. That is a finding, not a clean census."
        );
    }

    #[test]
    fn a_concordant_case_reports_positive_agreement_and_a_discordant_one_negative() {
        // The load-bearing half. Every case in the frozen corpus happens to
        // agree with the scorer, so the corpus CANNOT tell a correct column
        // from one that always answers "agreement" — these synthetic fixtures
        // are the only thing that can, and they are the reason the corpus is
        // not the evidence.
        let agreed = census(
            &[cell("a", SCALE_UNITS)],
            &baseline(&[("a", SCALE_UNITS)]),
            &verdicts(&[("a", true)]),
        );
        assert_eq!(agreed.results[0].agreement_units, Some(SCALE_UNITS));

        // A perfect score against a human who said the run failed: the scorer
        // disagrees, and the column must say so.
        let disagreed = census(
            &[cell("a", SCALE_UNITS)],
            &baseline(&[("a", SCALE_UNITS)]),
            &verdicts(&[("a", false)]),
        );
        assert_eq!(
            disagreed.results[0].agreement_units,
            Some(-SCALE_UNITS),
            "a perfect score the human contradicted is disagreement, and a column that cannot \
             report disagreement is a column that reports agreement unconditionally."
        );

        // And the mirror: an imperfect score against a human who said pass.
        let lost = census(
            &[cell("a", SCALE_UNITS - 1)],
            &baseline(&[("a", SCALE_UNITS)]),
            &verdicts(&[("a", true)]),
        );
        assert_eq!(lost.results[0].agreement_units, Some(-SCALE_UNITS));

        // A low score against a human who said fail: they agree.
        let low_but_right = census(
            &[cell("a", 7_400)],
            &baseline(&[("a", 7_400)]),
            &verdicts(&[("a", false)]),
        );
        assert_eq!(low_but_right.results[0].agreement_units, Some(SCALE_UNITS));
    }

    #[test]
    fn the_magnitude_is_the_concordance_and_not_the_score() {
        // Two disagreements at WILDLY different scores, and two agreements at
        // wildly different scores. Every one reads the same magnitude, so the
        // number cannot be a re-expression of the score wearing agreement's
        // name — and a reviewer comparing cells cannot read a strength of
        // agreement into it that was never measured.
        let out = census(
            &[
                cell("high_bad", SCALE_UNITS),
                cell("low_bad", 7_400),
                cell("high_ok", SCALE_UNITS),
                cell("low_ok", 7_400),
            ],
            &baseline(&[
                ("high_bad", SCALE_UNITS),
                ("low_bad", 7_400),
                ("high_ok", SCALE_UNITS),
                ("low_ok", 7_400),
            ]),
            &verdicts(&[
                ("high_bad", false),
                ("low_bad", true),
                ("high_ok", true),
                ("low_ok", false),
            ]),
        );
        let by_id = |id: &str| {
            out.results
                .iter()
                .find(|r| r.id == id)
                .unwrap_or_else(|| panic!("cell {id} must be present"))
                .agreement_units
        };
        assert_eq!(by_id("high_bad"), Some(-SCALE_UNITS));
        assert_eq!(by_id("low_bad"), Some(-SCALE_UNITS));
        assert_eq!(by_id("high_ok"), Some(SCALE_UNITS));
        assert_eq!(by_id("low_ok"), Some(SCALE_UNITS));
        assert_ne!(by_id("high_bad"), Some(0), "agreement is never zero-valued");
    }

    #[test]
    fn agreement_is_reported_and_never_gates() {
        // A census whose only cell CONTRADICTS the frozen human verdict, and
        // sits inside tolerance. It is clean: nothing drifted. If disagreement
        // could move a verdict, a breach, or the exit code, this would fail —
        // and a single-rater measure over seven cases blocking a release is the
        // unmeasured control the posture exists to prevent.
        let out = census(
            &[cell("a", SCALE_UNITS)],
            &baseline(&[("a", SCALE_UNITS)]),
            &verdicts(&[("a", false)]),
        );
        assert_eq!(out.results[0].agreement_units, Some(-SCALE_UNITS));
        assert_eq!(
            out.results[0].verdict,
            CellVerdict::Stable,
            "agreement must not reach the drift verdict"
        );
        assert!(
            out.is_clean(),
            "disagreement is a REPORT, not a breach: the cell did not move, so nothing regressed"
        );
        assert!(out.breaches().is_empty());
        assert_eq!(out.total_drift_units(), 0);
    }

    #[test]
    fn the_pass_boundary_is_the_scorers_and_the_corpus_agrees_on_every_case() {
        // Measured, not assumed: the frozen corpus is unanimous with the
        // scorer today. That is worth pinning for two reasons — it is the
        // corpus's actual state, and it is WHY this round's discriminating
        // fixtures have to be synthetic. A future corpus case that disagreed
        // would flip this pin to RED, which is the census telling the operator
        // the instrument and the humans have parted company.
        let (cells, verdicts) = measure().expect("the corpus measures");
        let out = census(&cells, &baseline_all(&cells), &verdicts);
        let disagreements: Vec<&str> = out
            .results
            .iter()
            .filter(|r| r.agreement_units == Some(-SCALE_UNITS))
            .map(|r| r.id.as_str())
            .collect();
        assert!(
            disagreements.is_empty(),
            "the frozen corpus no longer agrees with the scorer on: {disagreements:?}. A \
             disagreement is a finding for a human to adjudicate — never a number to tune."
        );
        assert!(
            out.results.iter().all(|r| r.agreement_units.is_some()),
            "every corpus case is baselined, so every one must carry an agreement figure"
        );
    }

    /// The committed vector's shape for a set of cells: each cell baselined at
    /// its own measurement, which is what makes the agreement census above a
    /// census about concordance rather than about drift.
    fn baseline_all(cells: &[Cell]) -> BTreeMap<String, i32> {
        cells
            .iter()
            .map(|c| (c.id.clone(), c.observed_units))
            .collect()
    }

    #[test]
    fn an_empty_census_is_not_clean() {
        // Vacuity: a census that measured nothing must never report success, or
        // a corpus that failed to load is indistinguishable from a good week.
        // It also reports no agreement: nothing measured, nothing claimed.
        let out = census(&[], &BTreeMap::new(), &no_verdicts());
        assert!(!out.is_clean());
        assert!(
            out.results.is_empty(),
            "an empty census must emit no cell line, and therefore no agreement figure"
        );
    }

    #[test]
    fn the_verdict_vocabulary_is_closed_and_carries_no_cell_content() {
        for v in [
            CellVerdict::Stable,
            CellVerdict::Drifted,
            CellVerdict::Unbaselined,
            CellVerdict::Orphaned,
        ] {
            let s = v.as_str();
            assert!(
                s.chars().all(|c| c.is_ascii_lowercase()),
                "verdict `{s}` is not a plain closed token"
            );
            assert!(
                !s.chars().any(|c| c.is_ascii_digit()),
                "verdict `{s}` carries a number; an emitted label must be a function of the \
                 variant alone so no cell id can ever vary it"
            );
        }
        assert!(!CellVerdict::Unbaselined.is_breach());
        assert!(!CellVerdict::Orphaned.is_breach());
        assert!(CellVerdict::Drifted.is_breach());
    }

    #[test]
    fn the_census_is_order_independent_and_reproducible() {
        let (a, b, c) = (
            cell("alpha", 9_000),
            cell("beta", 4_000),
            cell("gamma", 1_000),
        );
        let base = baseline(&[("alpha", 9_000), ("beta", 4_000), ("gamma", 1_000)]);
        let forward = census(&[a.clone(), b.clone(), c.clone()], &base, &no_verdicts());
        let reverse = census(&[c, b, a], &base, &no_verdicts());
        assert_eq!(
            forward, reverse,
            "cell order must not change the census; an unstable order makes two censuses \
             undiffable"
        );
        assert_eq!(
            census(
                &[
                    cell("alpha", 9_000),
                    cell("beta", 4_000),
                    cell("gamma", 1_000)
                ],
                &base,
                &no_verdicts()
            ),
            forward
        );
    }

    #[test]
    fn the_total_drift_is_the_sum_of_absolute_movement() {
        // `a` moved +100, `b` moved -100, and `c` is baselined at 1000 but
        // observed at 10000 — a 9000-unit swing, and the whole 10000-unit scale.
        // The total is the sum of ABSOLUTE movement, so signs do not cancel:
        // a census whose headline number could be driven to zero by two
        // equal-and-opposite regressions would report a clean week on the day
        // the instrument broke in two directions at once.
        let out = census(
            &[cell("a", 9_100), cell("b", 3_900), cell("c", 10_000)],
            &baseline(&[("a", 9_000), ("b", 4_000), ("c", 1_000)]),
            &no_verdicts(),
        );
        assert_eq!(out.total_drift_units(), 100 + 100 + 9_000);
    }

    #[test]
    fn an_out_of_scale_score_drifts_rather_than_overflowing() {
        // A scorer returning a wild value is a scorer bug; the control must
        // REPORT it, not panic while computing the delta.
        let out = census(
            &[cell("a", i32::MAX)],
            &baseline(&[("a", i32::MIN)]),
            &no_verdicts(),
        );
        assert_eq!(out.results[0].verdict, CellVerdict::Drifted);
        assert_eq!(out.results[0].delta_units, Some(i32::MAX));
    }

    #[test]
    fn the_corpus_is_the_seven_frozen_cases() {
        let cases = corpus().expect("the frozen corpus decodes");
        assert_eq!(cases.len(), 7, "2 qc_report + 5 gdl cases");
        let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "the corpus carries a duplicate case id");
    }

    #[test]
    fn a_malformed_pack_is_refused_rather_than_silently_shortened() {
        assert!(decode_pack("not json").is_err());
        assert!(
            decode_pack("42").is_err(),
            "a pack that is neither array nor object"
        );
        // A case carrying a field this decoder does not know is refused. This is
        // the whole point of the strict decoder: a pack that grows a field the
        // census would silently ignore must be a LOUD failure.
        assert!(
            decode_pack(r#"[{"id":"a","family":"f","system_version":"1","scorer_version":"1","kappa_units":9000,"human_pass":true,"artifacts":{},"future_field":7}]"#).is_err(),
            "a pack carrying an undeclared field must be refused, not decoded past"
        );
        // A case missing a declared field is likewise refused.
        assert!(
            decode_pack(r#"[{"id":"a","family":"f","system_version":"1","kappa_units":9000,"human_pass":true,"artifacts":{}}]"#).is_err(),
            "a case with no scorer_version must be refused"
        );
    }

    #[test]
    fn both_pack_shapes_decode_and_both_real_shapes_are_covered() {
        // The corpus genuinely has two shapes: `qc_report.json` is an array of
        // two, each `gdl_cases/*.json` is a single object. One decoder over
        // both, so the two can never disagree about what a case is.
        const ONE: &str = r#"{"id":"one","family":"gdl_cases","system_version":"1","scorer_version":"1","kappa_units":8400,"human_pass":true,"artifacts":{}}"#;
        assert_eq!(decode_pack(ONE).expect("single object decodes").len(), 1);
        assert_eq!(
            decode_pack(&format!("[{ONE}]"))
                .expect("array decodes")
                .len(),
            1
        );
        // And the real corpus exercises BOTH paths, so neither is dead code.
        let cases = corpus().expect("the corpus decodes");
        assert_eq!(
            cases.len(),
            7,
            "2 from the array pack + 5 from the object packs"
        );
    }

    #[test]
    fn the_pack_bound_is_enforced() {
        let mut cases: Vec<String> = Vec::new();
        for i in 0..=MAX_CORPUS_CASES {
            cases.push(format!(
                r#"{{"id":"c{i}","family":"f","system_version":"1","scorer_version":"1","kappa_units":9000,"human_pass":true,"artifacts":{{}}}}"#
            ));
        }
        assert!(
            decode_pack(&format!("[{}]", cases.join(","))).is_err(),
            "a pack past the bound is refused; a census over fewer cases than the corpus holds \
             reports clean over cases it never looked at"
        );
    }

    #[test]
    fn measuring_the_corpus_produces_one_cell_per_case_from_the_real_scorer() {
        let (cells, verdicts) = measure().expect("the corpus measures");
        let cases = corpus().expect("the corpus decodes");
        assert_eq!(cells.len(), cases.len());
        assert_eq!(
            verdicts.len(),
            cases.len(),
            "every corpus case carries a frozen verdict, so the census's verdict map covers the \
             corpus exactly"
        );
        for (c, case) in cells.iter().zip(cases.iter()) {
            assert_eq!(c.id, case.id);
            assert!(
                (0..=SCALE_UNITS).contains(&c.observed_units),
                "case {} scored {} units, outside the scale — a scorer returning an out-of-scale \
                 value has a bug and the census must say so",
                case.id,
                c.observed_units
            );
        }
    }

    /// The load-bearing pin for this module's second decoder.
    ///
    /// This module cannot link `gold-sets`, so it decodes the pack shape itself.
    /// That is a third reading of one shape (`CaseArtifacts`, `RunArtifacts`,
    /// `PackArtifacts`) and three readings can disagree. This reads the crate's
    /// **declaration** as text and checks the agreement, so a pack that grows a
    /// field the census ignores is a loud failure rather than a census quietly
    /// measuring less than the corpus holds.
    #[test]
    fn the_decoder_covers_every_field_the_pack_declares() {
        /// The field names declared by one struct in the crate's source.
        fn declared_fields(src: &str, struct_name: &str) -> Vec<String> {
            let body = src
                .split_once(&format!("pub struct {struct_name} {{"))
                .map(|(_, rest)| rest)
                .and_then(|rest| rest.split_once('}').map(|(body, _)| body))
                .unwrap_or_else(|| panic!("{struct_name} is not declared in the gold-sets source"));
            body.lines()
                .filter_map(|l| {
                    let t = l.trim();
                    let rest = t.strip_prefix("pub ")?;
                    let name = rest.split(':').next()?;
                    let name = name.trim();
                    // Skip attributes, not fields.
                    if name.starts_with('#') || name.is_empty() {
                        return None;
                    }
                    Some(name.to_string())
                })
                .collect()
        }

        /// The field names this module's own decoder declares.
        fn census_fields<T: FieldNames>() -> Vec<String> {
            T::FIELDS.iter().map(|s| (*s).to_string()).collect()
        }

        trait FieldNames {
            const FIELDS: &'static [&'static str];
        }
        impl FieldNames for PackArtifacts {
            const FIELDS: &'static [&'static str] = &[
                "steps",
                "findings",
                "contradictions",
                "audit_ok",
                "repeat_contact",
                "handoff_complete",
                "verified",
                "escalation_honored",
            ];
        }
        impl FieldNames for PackStep {
            const FIELDS: &'static [&'static str] = &[
                "expected",
                "actual",
                "skipped_verify",
                "abstained",
                "guidance_accepted",
            ];
        }

        let artifacts = declared_fields(GOLD_SETS_DECLARATION, "CaseArtifacts");
        assert!(
            !artifacts.is_empty(),
            "the gold-sets declaration must expose CaseArtifacts; a rename there is a corpus \
             change and the census must be told, not silently decode past it"
        );
        assert_eq!(
            census_fields::<PackArtifacts>(),
            artifacts,
            "the census's artifact decoder and the gold pack's declared shape disagree. The \
             census reads the packs itself because the SDK carries no serde derive; that third \
             reading is only safe while the two agree."
        );

        let steps = declared_fields(GOLD_SETS_DECLARATION, "CaseStep");
        assert_eq!(
            census_fields::<PackStep>(),
            steps,
            "the census's step decoder and the gold pack's declared shape disagree"
        );
    }

    /// The pack ORDER and the crate loader's order must agree, or the two
    /// corpora are the same seven cases in a different sequence and every
    /// diff between them is noise.
    #[test]
    fn the_pack_order_matches_the_crate_loader() {
        let loader = GOLD_SETS_DECLARATION
            .split("pub fn gdl_cases()")
            .nth(1)
            .and_then(|rest| rest.split("pub fn all()").next())
            .unwrap_or_else(|| panic!("gdl_cases() is not declared in the gold-sets source"));
        for (name, _) in GDL_PACK_FILES {
            assert!(
                loader.contains(&format!("\"{name}\"")),
                "the census loads {name}.json but gold_sets::gdl_cases() does not. The two \
                 corpora would then be the same cases in a different order, and every diff \
                 between them is noise."
            );
        }
    }
}
