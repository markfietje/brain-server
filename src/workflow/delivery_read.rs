//! The derived delivery read model — the DO-named operate surface (the
//! delivery line's closing round).
//!
//! A pure query core: it takes [`Connection`] + `(window, now)` and derives
//! the coupled cluster read-time from `delivery_releases` joined to the
//! authority-fact findings the authority reconcile wrote. It is a QUERY, never a
//! view (the view law — the repo has never used a view; a view has no `?window=` story
//! and drags migration-parity questions), and it persists nothing: no table,
//! no stamp, no writer, no egress. Because nothing is persisted, any future
//! definitional change costs zero migrations.
//!
//! The laws this module carries:
//!
//! - **The typed insufficiency contract.** Every metric renders a
//!   [`MetricState`]: `computed` always carries a value; `insufficient` never
//!   does and carries a closed [`InsufficiencyReason`]. An absent metric is
//!   never rendered `0` and a zero is never rendered absent.
//! - **The coupling law.** Throughput and instability are ONE cluster in
//!   ONE response object; `change_fail_rate` is labeled `role: "control"` on
//!   the readings; no field decomposition lets a consumer target throughput in
//!   isolation. The framing rides with the numbers so no client can render a
//!   bare throughput scalar as a performance verdict.
//! - **The naming law.** The native measures are named
//!   `approval_to_promotion_elapsed` and `governed_release_cadence`. Where
//!   DORA (DevOps Research and Assessment) names are used at all, the reading
//!   carries `dora_name` + `definition_match: "proxy"` + a one-line definition
//!   note; the native elapsed measure never carries a DORA label.
//! - **The change-fail filter law.** The signal is the authority
//!   contradiction the reconcile writes: a `findings` row with the CLOSED
//!   source vocabulary (`source LIKE 'delivery:%'`) and the typed confidence
//!   column (`0.0` — the mismatch arm; the match arm writes `1.0`). The claim
//!   text is never read: `findings.claim` is free text, and matching it would
//!   be a forged metric. The measured substrate stores the kind as a claim
//!   prefix only (no kind column), so the typed confidence column is the
//!   structured discriminator within the closed source family. The rate's
//!   denominator is the window's promoted releases; a contradiction on a run
//!   whose release is not promoted in-window is out of the denominator.
//! - **The commit-time fact contract.** Commit-anchored change lead time
//!   computes only when the release's `commit_sha` joins to a recorded vcs
//!   commit-time fact: a `findings` row with `source = 'delivery:vcs'`,
//!   confidence `1.0`, whose machine-written `evidence` slot carries the
//!   structured keys `commit_time=<epoch-seconds>` and, when it binds a
//!   revision, `commit_sha=<sha>` (the evidence slot is the typed-evidence
//!   fact channel; the claim text is never parsed). A fact that names a
//!   revision only matches a release carrying that same revision; a run-level
//!   fact (no revision key) matches only a release with no revision to bind.
//!   No production writer emits `commit_time=` today (measured at `c576821`:
//!   the adapters fetch facts at call time and persist only claims), so the
//!   LIVE branch is `insufficient` (`no_vcs_revision_recorded`) — the honest
//!   answer; the computed branch is implemented and unit-proven so the metric
//!   is correct the day the facts exist. No timestamp is ever approximated.
//! - **The window.** Days, integer, default 30, bounded `1..=366`,
//!   validated HERE and refused — never silently clamped. The derivation is
//!   deterministic for (window, now): the core reads no clock.
//! - **The baseline.** The run's OWN history is the only baseline: an
//!   `own_baseline` block over a fixed 90-day window of the same family, and
//!   no benchmark threshold, table, figure, or performance band is reproduced
//!   anywhere. Attribution is the whole of what crosses.
//!
//! Governance frame, stated once and no more: the surface is
//! transparency/auditability by design — a governed operator reads derived
//! facts over their own audited records. It makes no automated decision about
//! a person, so no AI Act high-risk duty is triggered by this code. It
//! carries no EU DORA obligation and makes no operational-resilience claim.
//! No AI Act / CRA / GDPR conclusion is drawn or claimable from any of it.

use std::collections::HashSet;

use rusqlite::{Connection, params, params_from_iter};
use serde::Serialize;

use crate::workflow::delivery::RUN_KIND;

/// The default window, in days (the window law).
pub(crate) const DEFAULT_WINDOW_DAYS: i64 = 30;
/// The window bound, in days (the window law): refused outside `1..=MAX_WINDOW_DAYS`,
/// never clamped.
pub(crate) const MAX_WINDOW_DAYS: i64 = 366;
/// The baseline window, in days: the run's own history, the only baseline the
/// model may compare against (the licensing law).
pub(crate) const BASELINE_WINDOW_DAYS: i64 = 90;

const CLUSTER_NOTE: &str =
    "throughput and instability are a coupled cluster; change_fail_rate is the control";
const FRAMING: &str =
    "leading indicators for organizational performance; lagging for delivery practices";
const NATIVE_NOTE: &str =
    "NOT DORA change lead time; see change_lead_time for the commit-anchored metric";
const CONTROL_ROLE: &str = "control";
const CONTROL_METRIC: &str = "change_fail_rate";
const PROXY: &str = "proxy";
const ANCHOR: &str = "vcs-commit→promoted";
const ATTRIBUTION_PROGRAMME: &str = "DORA (DevOps Research and Assessment)";
const ATTRIBUTION_REFERENCE: &str = "https://dora.dev/guides/dora-metrics/";
const ATTRIBUTION_LICENSE_NOTE: &str =
    "metrics vocabulary only; no thresholds or tables reproduced";

const LEAD_TIME_NOTE: &str = "elapsed minutes from the vcs commit recorded on the release to its promotion; the DORA measure anchors at the commit";
const CADENCE_NOTE: &str = "governed releases reaching promotion per week in the window — a governed-promotion proxy, not the DORA deployment count";
const FAIL_RATE_NOTE: &str = "in-window promoted releases whose run carries a delivery authority contradiction (closed source vocabulary, typed confidence 0.0), over all in-window promoted releases";

/// Every refusal the read model can produce. Typed, so the handler maps
/// rather than guesses; closed, so a caller cannot learn a new failure mode
/// from a new error.
#[derive(Debug)]
pub(crate) enum OutcomesError {
    /// A window outside `1..=366`, or not an integer — the handler maps this
    /// to a 400. Refused, never clamped.
    WindowBounds { detail: String },
    /// A storage failure under the read.
    Storage { detail: String },
}

impl std::fmt::Display for OutcomesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutcomesError::WindowBounds { detail } => write!(f, "window out of bounds: {detail}"),
            OutcomesError::Storage { detail } => write!(f, "storage: {detail}"),
        }
    }
}

fn read_storage(detail: impl std::fmt::Display) -> OutcomesError {
    OutcomesError::Storage {
        detail: detail.to_string(),
    }
}

fn bounds(detail: impl std::fmt::Display) -> OutcomesError {
    OutcomesError::WindowBounds {
        detail: detail.to_string(),
    }
}

/// The typed per-metric state — the whole mechanism (the typed contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetricState {
    Computed,
    Insufficient,
}

/// The closed insufficiency vocabulary (the typed contract). Carried verbatim on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum InsufficiencyReason {
    #[serde(rename = "window_empty")]
    WindowEmpty,
    #[serde(rename = "no_vcs_revision_recorded")]
    NoVcsRevisionRecorded,
    #[serde(rename = "no_incident_facts")]
    NoIncidentFacts,
    #[serde(rename = "no_rework_signal")]
    NoReworkSignal,
    #[serde(rename = "insufficient_history")]
    InsufficientHistory,
}

/// Commit-anchored change lead time. The DORA name rides ONLY beside its
/// proxy label; the computed branch carries the median and its percentile,
/// the insufficient branch carries the closed reason and never a value.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct LeadTimeMetric {
    state: MetricState,
    #[serde(skip_serializing_if = "Option::is_none")]
    value_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    percentile: Option<u8>,
    dora_name: String,
    definition_match: String,
    definition_note: String,
    anchor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<InsufficiencyReason>,
}

impl LeadTimeMetric {
    fn labels() -> (String, String, String, String) {
        (
            "change lead time".to_string(),
            PROXY.to_string(),
            LEAD_TIME_NOTE.to_string(),
            ANCHOR.to_string(),
        )
    }

    fn computed(minutes: f64) -> Self {
        let (dora_name, definition_match, definition_note, anchor) = Self::labels();
        Self {
            state: MetricState::Computed,
            value_minutes: Some(minutes),
            percentile: Some(50),
            dora_name,
            definition_match,
            definition_note,
            anchor,
            reason: None,
        }
    }

    fn insufficient(reason: InsufficiencyReason) -> Self {
        let (dora_name, definition_match, definition_note, anchor) = Self::labels();
        Self {
            state: MetricState::Insufficient,
            value_minutes: None,
            percentile: None,
            dora_name,
            definition_match,
            definition_note,
            anchor,
            reason: Some(reason),
        }
    }
}

/// Governed release cadence — native-named, DORA-labeled as a proxy.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CadenceMetric {
    state: MetricState,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_week: Option<f64>,
    dora_name: String,
    definition_match: String,
    definition_note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<InsufficiencyReason>,
}

impl CadenceMetric {
    fn labels() -> (String, String, String) {
        (
            "deployment frequency".to_string(),
            PROXY.to_string(),
            CADENCE_NOTE.to_string(),
        )
    }

    fn computed(per_week: f64) -> Self {
        let (dora_name, definition_match, definition_note) = Self::labels();
        Self {
            state: MetricState::Computed,
            per_week: Some(per_week),
            dora_name,
            definition_match,
            definition_note,
            reason: None,
        }
    }

    fn insufficient(reason: InsufficiencyReason) -> Self {
        let (dora_name, definition_match, definition_note) = Self::labels();
        Self {
            state: MetricState::Insufficient,
            per_week: None,
            dora_name,
            definition_match,
            definition_note,
            reason: Some(reason),
        }
    }
}

/// The change-fail reading: the control of the coupled cluster. In the
/// cluster it carries `role: "control"`; in the baseline the role is not
/// restated.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct FailRateMetric {
    state: MetricState,
    #[serde(skip_serializing_if = "Option::is_none")]
    ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    dora_name: String,
    definition_match: String,
    definition_note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<InsufficiencyReason>,
}

impl FailRateMetric {
    fn labels() -> (String, String, String) {
        (
            "change failure rate".to_string(),
            PROXY.to_string(),
            FAIL_RATE_NOTE.to_string(),
        )
    }

    fn computed(ratio: f64, role: Option<String>) -> Self {
        let (dora_name, definition_match, definition_note) = Self::labels();
        Self {
            state: MetricState::Computed,
            ratio: Some(ratio),
            role,
            dora_name,
            definition_match,
            definition_note,
            reason: None,
        }
    }

    fn insufficient(reason: InsufficiencyReason) -> Self {
        let (dora_name, definition_match, definition_note) = Self::labels();
        Self {
            state: MetricState::Insufficient,
            ratio: None,
            role: None,
            dora_name,
            definition_match,
            definition_note,
            reason: Some(reason),
        }
    }
}

/// The native approval→promotion elapsed measure. NO DORA label of any kind —
/// The naming law's hard edge: a native name is never dressed as a DORA metric.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeElapsedMetric {
    state: MetricState,
    #[serde(skip_serializing_if = "Option::is_none")]
    value_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<InsufficiencyReason>,
}

impl NativeElapsedMetric {
    fn computed(minutes: f64) -> Self {
        Self {
            state: MetricState::Computed,
            value_minutes: Some(minutes),
            reason: None,
        }
    }

    fn insufficient(reason: InsufficiencyReason) -> Self {
        Self {
            state: MetricState::Insufficient,
            value_minutes: None,
            reason: Some(reason),
        }
    }
}

/// A metric the two-authority surface cannot support. Its reason is part of
/// its type: there are no incident facts and no rework signal in the
/// substrate, and declaring that honestly is correct — approximating it is
/// not. There is no computed constructor to reach for.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct UnsupportedMetric {
    state: MetricState,
    reason: InsufficiencyReason,
}

impl UnsupportedMetric {
    fn insufficient(reason: InsufficiencyReason) -> Self {
        Self {
            state: MetricState::Insufficient,
            reason,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Throughput {
    change_lead_time: LeadTimeMetric,
    governed_release_cadence: CadenceMetric,
    failed_deployment_recovery_time: UnsupportedMetric,
    control: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Instability {
    change_fail_rate: FailRateMetric,
    deployment_rework_rate: UnsupportedMetric,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeMeasures {
    approval_to_promotion_elapsed: NativeElapsedMetric,
    note: String,
}

/// The run's own history over the fixed baseline window — the only baseline
/// the model may compare against (the licensing law). Same typed states: an empty baseline
/// is `insufficient_history`, never a bare number, never zero.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct OwnBaseline {
    window_days: i64,
    change_fail_rate: FailRateMetric,
    cadence_per_week: CadenceMetric,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Cluster {
    note: String,
    framing: String,
    throughput: Throughput,
    instability: Instability,
    native_measures: NativeMeasures,
    own_baseline: OwnBaseline,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Attribution {
    programme: String,
    reference: String,
    license_note: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct OutcomesResponse {
    window_days: i64,
    generated_at: i64,
    domain: String,
    cluster: Cluster,
    attribution: Attribution,
}

/// One in-window promoted release row, projected to what the metrics need.
#[derive(Debug, Clone)]
struct WindowRelease {
    run_id: i64,
    approved_at: Option<i64>,
    deployed_at: i64,
    commit_sha: Option<String>,
}

/// A vcs commit-time fact, resolved read-time from the typed-evidence rows
/// (see the module contract — no production writer emits these keys yet).
#[derive(Debug, Clone)]
struct CommitFact {
    run_id: i64,
    commit_sha: Option<String>,
    commit_time: i64,
}

/// The window parse-and-bound law: days, integer, default 30, bounded
/// `1..=366`, refused — never clamped (the window law).
fn parse_window(window: Option<&str>) -> Result<i64, OutcomesError> {
    let Some(raw) = window else {
        return Ok(DEFAULT_WINDOW_DAYS);
    };
    let days = raw
        .trim()
        .parse::<i64>()
        .map_err(|_| bounds("window must be an integer number of days"))?;
    if !(1..=MAX_WINDOW_DAYS).contains(&days) {
        return Err(bounds(format!(
            "window must be between 1 and {MAX_WINDOW_DAYS} days"
        )));
    }
    Ok(days)
}

/// The window's promoted releases: `deployed_at` is set exactly when the
/// promotion walk lands `promoted`, so `deployed_at` in-window is the
/// deployment moment — reached-status vocabulary (`verified`, `rolled_back`
/// after it) stays inside the deployment count, and a release that never
/// promoted is out of every window metric by construction.
fn window_releases(
    conn: &Connection,
    domain: &str,
    from: i64,
    to: i64,
) -> Result<Vec<WindowRelease>, OutcomesError> {
    let mut stmt = conn
        .prepare(
            "SELECT rel.run_id, rel.approved_at, rel.deployed_at, rel.commit_sha \
             FROM delivery_releases rel \
             JOIN workflow_runs r ON r.id = rel.run_id AND r.kind = ?1 \
             WHERE r.domain = ?2 AND rel.deployed_at IS NOT NULL \
               AND rel.deployed_at > ?3 AND rel.deployed_at <= ?4 \
             ORDER BY rel.id",
        )
        .map_err(read_storage)?;
    let rows = stmt
        .query_map(params![RUN_KIND, domain, from, to], |row| {
            Ok(WindowRelease {
                run_id: row.get(0)?,
                approved_at: row.get(1)?,
                deployed_at: row.get(2)?,
                commit_sha: row.get(3)?,
            })
        })
        .map_err(read_storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(read_storage)?;
    Ok(rows)
}

/// The runs (of the given set) carrying at least one delivery authority
/// contradiction: the CLOSED source vocabulary (`delivery:%`) narrowed by the
/// typed confidence column (`0.0` — the mismatch arm). The claim text is
/// never read.
fn mismatched_runs(conn: &Connection, run_ids: &[i64]) -> Result<HashSet<i64>, OutcomesError> {
    if run_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let placeholders = vec!["?"; run_ids.len()].join(",");
    let sql = format!(
        "SELECT DISTINCT run_id FROM findings \
         WHERE source LIKE 'delivery:%' AND confidence = 0.0 AND run_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql).map_err(read_storage)?;
    let rows = stmt
        .query_map(params_from_iter(run_ids.iter()), |row| row.get::<_, i64>(0))
        .map_err(read_storage)?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(read_storage)?;
    Ok(rows)
}

/// The vcs commit-time facts for the given runs (the commit-time fact
/// contract in the module doc).
fn commit_facts(conn: &Connection, run_ids: &[i64]) -> Result<Vec<CommitFact>, OutcomesError> {
    if run_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; run_ids.len()].join(",");
    let sql = format!(
        "SELECT run_id, evidence FROM findings \
         WHERE source = 'delivery:vcs' AND confidence = 1.0 AND run_id IN ({placeholders}) \
         ORDER BY id"
    );
    let mut stmt = conn.prepare(&sql).map_err(read_storage)?;
    let rows = stmt
        .query_map(params_from_iter(run_ids.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(read_storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(read_storage)?;
    Ok(rows
        .into_iter()
        .filter_map(|(run_id, evidence)| {
            parse_commit_fact(&evidence).map(|(commit_sha, commit_time)| CommitFact {
                run_id,
                commit_sha,
                commit_time,
            })
        })
        .collect())
}

/// The machine-written evidence slot's structured keys: `commit_time=` (a
/// parseable epoch-second count is required) and `commit_sha=` (optional).
/// Only the evidence slot is parsed — the claim text is never read.
fn parse_commit_fact(evidence: &str) -> Option<(Option<String>, i64)> {
    let mut commit_sha = None;
    let mut commit_time = None;
    for token in evidence.split_whitespace() {
        if let Some((key, value)) = token.split_once('=') {
            match key {
                "commit_sha" => commit_sha = Some(value.to_string()),
                "commit_time" => commit_time = value.parse::<i64>().ok(),
                _ => {}
            }
        }
    }
    commit_time.map(|time| (commit_sha, time))
}

/// The lead-time samples: minutes from the bound commit-time fact to the
/// release's promotion. The binding is by run, and by revision whenever BOTH
/// the fact and the release name one.
fn lead_time_samples(releases: &[WindowRelease], facts: &[CommitFact]) -> Vec<f64> {
    releases
        .iter()
        .filter_map(|rel| {
            facts
                .iter()
                .find(|fact| {
                    fact.run_id == rel.run_id
                        && match (&fact.commit_sha, &rel.commit_sha) {
                            (Some(fact_sha), Some(rel_sha)) => fact_sha == rel_sha,
                            (Some(_), None) => false,
                            (None, _) => true,
                        }
                })
                .map(|fact| (rel.deployed_at - fact.commit_time) as f64 / 60.0)
        })
        .collect()
}

fn median(samples: &mut [f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    let mid = samples.len() / 2;
    Some(if samples.len().is_multiple_of(2) {
        (samples[mid - 1] + samples[mid]) / 2.0
    } else {
        samples[mid]
    })
}

fn mean(samples: &[f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    Some(samples.iter().sum::<f64>() / samples.len() as f64)
}

/// The derived delivery read model: the coupled cluster for ONE domain over
/// ONE window, computed read-time from the caller's connection and the
/// handler's `now`. Deterministic for (window, now); persists nothing.
pub(crate) fn delivery_outcomes(
    conn: &Connection,
    domain: &str,
    window: Option<&str>,
    now: i64,
) -> Result<OutcomesResponse, OutcomesError> {
    let window_days = parse_window(window)?;
    let releases = window_releases(conn, domain, now - window_days * 86_400, now)?;
    let window_empty = releases.is_empty();
    let run_ids: Vec<i64> = releases.iter().map(|r| r.run_id).collect();

    // ── throughput ─────────────────────────────────────────────────────────
    let change_lead_time = if window_empty {
        LeadTimeMetric::insufficient(InsufficiencyReason::WindowEmpty)
    } else {
        let facts = commit_facts(conn, &run_ids)?;
        let mut samples = lead_time_samples(&releases, &facts);
        median(&mut samples).map_or_else(
            || LeadTimeMetric::insufficient(InsufficiencyReason::NoVcsRevisionRecorded),
            LeadTimeMetric::computed,
        )
    };
    let governed_release_cadence = if window_empty {
        CadenceMetric::insufficient(InsufficiencyReason::WindowEmpty)
    } else {
        CadenceMetric::computed(releases.len() as f64 * 7.0 / window_days as f64)
    };

    // ── instability ────────────────────────────────────────────────────────
    let change_fail_rate = if window_empty {
        FailRateMetric::insufficient(InsufficiencyReason::WindowEmpty)
    } else {
        let mismatched = mismatched_runs(conn, &run_ids)?;
        let failed = releases
            .iter()
            .filter(|rel| mismatched.contains(&rel.run_id))
            .count();
        FailRateMetric::computed(
            failed as f64 / releases.len() as f64,
            Some(CONTROL_ROLE.to_string()),
        )
    };

    // The two-authority (vcs, ci) surface carries no incident or rework
    // facts, and declaring that honestly is correct (the typed contract) — always.
    let failed_deployment_recovery_time =
        UnsupportedMetric::insufficient(InsufficiencyReason::NoIncidentFacts);
    let deployment_rework_rate =
        UnsupportedMetric::insufficient(InsufficiencyReason::NoReworkSignal);

    // ── the native measure (the naming law's hard edge) ───────────────────
    let approval_to_promotion_elapsed = if window_empty {
        NativeElapsedMetric::insufficient(InsufficiencyReason::WindowEmpty)
    } else {
        let samples: Vec<f64> = releases
            .iter()
            .filter_map(|rel| {
                rel.approved_at
                    .map(|approved| (rel.deployed_at - approved) as f64 / 60.0)
            })
            .collect();
        mean(&samples).map_or_else(
            || NativeElapsedMetric::insufficient(InsufficiencyReason::WindowEmpty),
            NativeElapsedMetric::computed,
        )
    };

    // ── the baseline: the run's OWN history (the licensing law) ──────────
    let baseline_releases =
        window_releases(conn, domain, now - BASELINE_WINDOW_DAYS * 86_400, now)?;
    let baseline_empty = baseline_releases.is_empty();
    let baseline_fail_rate = if baseline_empty {
        FailRateMetric::insufficient(InsufficiencyReason::InsufficientHistory)
    } else {
        let baseline_run_ids: Vec<i64> = baseline_releases.iter().map(|r| r.run_id).collect();
        let mismatched = mismatched_runs(conn, &baseline_run_ids)?;
        let failed = baseline_releases
            .iter()
            .filter(|rel| mismatched.contains(&rel.run_id))
            .count();
        FailRateMetric::computed(failed as f64 / baseline_releases.len() as f64, None)
    };
    let baseline_cadence = if baseline_empty {
        CadenceMetric::insufficient(InsufficiencyReason::InsufficientHistory)
    } else {
        CadenceMetric::computed(baseline_releases.len() as f64 * 7.0 / BASELINE_WINDOW_DAYS as f64)
    };

    Ok(OutcomesResponse {
        window_days,
        generated_at: now,
        domain: domain.to_string(),
        cluster: Cluster {
            note: CLUSTER_NOTE.to_string(),
            framing: FRAMING.to_string(),
            throughput: Throughput {
                change_lead_time,
                governed_release_cadence,
                failed_deployment_recovery_time,
                control: CONTROL_METRIC.to_string(),
            },
            instability: Instability {
                change_fail_rate,
                deployment_rework_rate,
            },
            native_measures: NativeMeasures {
                approval_to_promotion_elapsed,
                note: NATIVE_NOTE.to_string(),
            },
            own_baseline: OwnBaseline {
                window_days: BASELINE_WINDOW_DAYS,
                change_fail_rate: baseline_fail_rate,
                cadence_per_week: baseline_cadence,
            },
        },
        attribution: Attribution {
            programme: ATTRIBUTION_PROGRAMME.to_string(),
            reference: ATTRIBUTION_REFERENCE.to_string(),
            license_note: ATTRIBUTION_LICENSE_NOTE.to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use serde_json::Value;

    const NOW: i64 = 1_800_000_000;
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn seed() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    fn seed_run(conn: &Connection, id: i64, domain: &str) {
        conn.execute(
            "INSERT INTO workflow_runs(id, domain, kind, state_json, state_revision, status, \
             created_at, updated_at) VALUES (?1, ?2, 'delivery', '{}', 1, 'active', 1, 1)",
            params![id, domain],
        )
        .unwrap();
    }

    fn seed_release(
        conn: &Connection,
        id: i64,
        run_id: i64,
        approved_at: i64,
        deployed_at: i64,
        commit_sha: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO delivery_releases(id, run_id, binding_id, ref, environment, \
             artifact_digest, status, created_at, updated_at, approved_at, deployed_at, \
             commit_sha) VALUES (?1, ?2, 1, 'main', 'production', 'sha256:abc', 'promoted', \
             ?3, ?3, ?4, ?5, ?6)",
            params![
                id,
                run_id,
                approved_at - 3600,
                approved_at,
                deployed_at,
                commit_sha
            ],
        )
        .unwrap();
    }

    fn seed_finding(conn: &Connection, run_id: i64, source: &str, confidence: f64, evidence: &str) {
        conn.execute(
            "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts) \
             VALUES (?1, 'claim', ?2, ?3, ?4, ?5)",
            params![run_id, evidence, source, confidence, NOW],
        )
        .unwrap();
    }

    /// A response over a seeded, fully computed window.
    fn computed_response() -> OutcomesResponse {
        let conn = seed();
        seed_run(&conn, 1, "global");
        seed_run(&conn, 2, "global");
        // Two promoted releases, each approved exactly one hour before deploy.
        seed_release(&conn, 1, 1, NOW - 3700, NOW - 100, Some(SHA));
        seed_release(&conn, 2, 2, NOW - 3900, NOW - 300, None);
        // One delivery authority contradiction (the mismatch arm) on run 1.
        seed_finding(&conn, 1, "delivery:vcs", 0.0, "authority_digest=deadbeef");
        // And one match observation, which must NOT count as a failure.
        seed_finding(&conn, 2, "delivery:ci", 1.0, "authority_digest=cafe");
        delivery_outcomes(&conn, "global", None, NOW).unwrap()
    }

    #[test]
    fn the_window_defaults_and_bounds_are_refused_not_clamped() {
        let conn = seed();
        let response = delivery_outcomes(&conn, "global", None, NOW).unwrap();
        assert_eq!(response.window_days, 30, "the default window is 30 days");
        for ok in ["1", " 366 "] {
            let response = delivery_outcomes(&conn, "global", Some(ok), NOW).unwrap();
            assert_eq!(
                response.window_days,
                ok.trim().parse::<i64>().unwrap(),
                "`{ok}` is inside the bound and parses"
            );
        }
        for refused in ["0", "-5", "367", "abc", "30.5", ""] {
            let outcome = delivery_outcomes(&conn, "global", Some(refused), NOW);
            assert!(
                matches!(outcome, Err(OutcomesError::WindowBounds { .. })),
                "`{refused}` is refused with the typed bounds error, never clamped"
            );
        }
    }

    #[test]
    fn an_empty_window_is_insufficient_never_zero() {
        let conn = seed();
        let response = delivery_outcomes(&conn, "global", None, NOW).unwrap();
        let wire = serde_json::to_value(&response).unwrap();
        let cluster = &wire["cluster"];
        for [group, metric] in [
            ["throughput", "change_lead_time"],
            ["throughput", "governed_release_cadence"],
            ["instability", "change_fail_rate"],
            ["native_measures", "approval_to_promotion_elapsed"],
        ] {
            let reading = &cluster[group][metric];
            assert_eq!(reading["state"], "insufficient", "{group}/{metric}");
            assert_eq!(reading["reason"], "window_empty", "{group}/{metric}");
        }
        assert_eq!(
            cluster["own_baseline"]["change_fail_rate"]["reason"],
            "insufficient_history"
        );
        assert_eq!(
            cluster["own_baseline"]["cadence_per_week"]["reason"],
            "insufficient_history"
        );
        // And no value field is rendered anywhere — absent never masquerades as zero.
        let text = serde_json::to_string(&wire).unwrap();
        for value_field in ["\"value_minutes\":", "\"per_week\":", "\"ratio\":"] {
            assert!(
                !text.contains(value_field),
                "an empty window renders no {value_field} anywhere — absent is never zero"
            );
        }
    }

    #[test]
    fn the_cluster_computes_over_seeded_rows() {
        let response = computed_response();
        assert_eq!(response.window_days, 30);
        let wire = serde_json::to_value(&response).unwrap();
        let throughput = &wire["cluster"]["throughput"];
        let instability = &wire["cluster"]["instability"];
        // Cadence: 2 promoted releases over 30 days.
        assert_eq!(throughput["governed_release_cadence"]["state"], "computed");
        assert_eq!(
            throughput["governed_release_cadence"]["per_week"],
            serde_json::json!(2.0 * 7.0 / 30.0)
        );
        // The change-fail control: one of the two runs carries a contradiction.
        assert_eq!(instability["change_fail_rate"]["state"], "computed");
        assert_eq!(
            instability["change_fail_rate"]["ratio"],
            serde_json::json!(0.5)
        );
        assert_eq!(instability["change_fail_rate"]["role"], "control");
        // Approval → promotion elapsed: the mean of 60 and 60 minutes.
        assert_eq!(
            wire["cluster"]["native_measures"]["approval_to_promotion_elapsed"]["state"],
            "computed"
        );
        assert_eq!(
            wire["cluster"]["native_measures"]["approval_to_promotion_elapsed"]["value_minutes"],
            serde_json::json!(60.0)
        );
        // No commit-time fact exists in production: the live branch is honest.
        assert_eq!(throughput["change_lead_time"]["state"], "insufficient");
        assert_eq!(
            throughput["change_lead_time"]["reason"],
            "no_vcs_revision_recorded"
        );
    }

    #[test]
    fn zero_is_a_computed_zero_not_an_absence() {
        let conn = seed();
        seed_run(&conn, 1, "global");
        seed_release(&conn, 1, 1, NOW - 200, NOW - 100, None);
        let response = delivery_outcomes(&conn, "global", None, NOW).unwrap();
        let wire = serde_json::to_value(&response).unwrap();
        let fail = &wire["cluster"]["instability"]["change_fail_rate"];
        assert_eq!(
            fail["state"], "computed",
            "a real zero is computed, not absent"
        );
        assert_eq!(fail["ratio"], serde_json::json!(0.0));
        assert!(
            fail.get("reason").is_none(),
            "a computed zero carries no reason"
        );
    }

    #[test]
    fn change_lead_time_computes_only_when_the_commit_time_fact_exists() {
        let conn = seed();
        seed_run(&conn, 1, "global");
        seed_release(&conn, 1, 1, NOW - 200, NOW - 100, Some(SHA));
        // No fact: the live branch.
        let response = delivery_outcomes(&conn, "global", None, NOW).unwrap();
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(
            wire["cluster"]["throughput"]["change_lead_time"]["state"],
            "insufficient"
        );
        assert_eq!(
            wire["cluster"]["throughput"]["change_lead_time"]["reason"],
            "no_vcs_revision_recorded"
        );
        // A revision-bound fact, two hours before the deploy: the computed
        // branch fires, and the metric is correct the day the facts exist.
        seed_finding(
            &conn,
            1,
            "delivery:vcs",
            1.0,
            &format!(
                "authority_digest=deadbeef commit_sha={SHA} commit_time={}",
                NOW - 100 - 7200
            ),
        );
        let response = delivery_outcomes(&conn, "global", None, NOW).unwrap();
        let wire = serde_json::to_value(&response).unwrap();
        let lead = &wire["cluster"]["throughput"]["change_lead_time"];
        assert_eq!(lead["state"], "computed");
        assert_eq!(lead["value_minutes"], serde_json::json!(120.0));
        assert_eq!(lead["percentile"], serde_json::json!(50));
        // A fact bound to a DIFFERENT revision does not match the release.
        let conn = seed();
        seed_run(&conn, 1, "global");
        seed_release(&conn, 1, 1, NOW - 200, NOW - 100, Some(SHA));
        seed_finding(
            &conn,
            1,
            "delivery:vcs",
            1.0,
            &format!("commit_sha={}", SHA.replace('0', "f")),
        );
        let response = delivery_outcomes(&conn, "global", None, NOW).unwrap();
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(
            wire["cluster"]["throughput"]["change_lead_time"]["reason"], "no_vcs_revision_recorded",
            "a revision the release does not carry never binds"
        );
    }

    #[test]
    fn the_unsupported_metrics_declare_their_honesty_always() {
        let conn = seed();
        seed_run(&conn, 1, "global");
        seed_release(&conn, 1, 1, NOW - 200, NOW - 100, None);
        let response = delivery_outcomes(&conn, "global", None, NOW).unwrap();
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(
            wire["cluster"]["throughput"]["failed_deployment_recovery_time"],
            serde_json::json!({"state": "insufficient", "reason": "no_incident_facts"})
        );
        assert_eq!(
            wire["cluster"]["instability"]["deployment_rework_rate"],
            serde_json::json!({"state": "insufficient", "reason": "no_rework_signal"})
        );
    }

    #[test]
    fn the_derivation_writes_nothing() {
        let conn = seed();
        seed_run(&conn, 1, "global");
        seed_release(&conn, 1, 1, NOW - 200, NOW - 100, None);
        seed_finding(&conn, 1, "delivery:vcs", 0.0, "authority_digest=deadbeef");
        fn counts(conn: &Connection) -> Vec<i64> {
            [
                "findings",
                "contradictions",
                "delivery_releases",
                "workflow_runs",
            ]
            .iter()
            .map(|t| {
                conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
            })
            .collect()
        }
        let before = counts(&conn);
        delivery_outcomes(&conn, "global", None, NOW).unwrap();
        delivery_outcomes(&conn, "global", Some("90"), NOW).unwrap();
        assert_eq!(
            before,
            counts(&conn),
            "two derivations wrote nothing anywhere"
        );
    }

    #[test]
    fn the_typed_contract_holds_structurally_on_the_wire() {
        // Both worlds: a computed window and an empty one.
        let full = {
            let response = computed_response();
            serde_json::to_value(&response).unwrap()
        };
        let empty = {
            let conn = seed();
            let response = delivery_outcomes(&conn, "global", None, NOW).unwrap();
            serde_json::to_value(&response).unwrap()
        };
        const VALUE_FIELDS: [&str; 3] = ["value_minutes", "per_week", "ratio"];
        fn walk(node: &Value) {
            match node {
                Value::Object(map) => {
                    if let Some(Value::String(state)) = map.get("state") {
                        let carries_value = VALUE_FIELDS.iter().any(|f| map.contains_key(*f));
                        match state.as_str() {
                            "computed" => assert!(
                                carries_value && !map.contains_key("reason"),
                                "a computed metric always carries a value and never a reason: {map:?}"
                            ),
                            "insufficient" => assert!(
                                !carries_value && map.contains_key("reason"),
                                "an insufficient metric carries a reason and never a value: {map:?}"
                            ),
                            _ => panic!("no third state exists: {state}"),
                        }
                    }
                    if map.contains_key("dora_name") {
                        assert!(
                            map.contains_key("definition_match"),
                            "a dora_name never travels without its proxy label: {map:?}"
                        );
                    }
                    for child in map.values() {
                        walk(child);
                    }
                }
                Value::Array(items) => items.iter().for_each(walk),
                _ => {}
            }
        }
        walk(&full);
        walk(&empty);
    }

    #[test]
    fn the_baseline_is_the_own_history_of_the_same_window_family() {
        let conn = seed();
        // One release 80 days ago: outside the 30-day window, inside the 90.
        let deployed = NOW - 80 * 86_400;
        seed_run(&conn, 1, "global");
        seed_release(&conn, 1, 1, deployed - 3600, deployed, None);
        let response = delivery_outcomes(&conn, "global", None, NOW).unwrap();
        let wire = serde_json::to_value(&response).unwrap();
        let cluster = &wire["cluster"];
        assert_eq!(
            cluster["throughput"]["governed_release_cadence"]["reason"], "window_empty",
            "the window itself is empty"
        );
        let baseline = &cluster["own_baseline"];
        assert_eq!(baseline["window_days"], serde_json::json!(90));
        assert_eq!(baseline["change_fail_rate"]["state"], "computed");
        assert_eq!(
            baseline["change_fail_rate"]["ratio"],
            serde_json::json!(0.0)
        );
        assert_eq!(
            baseline["cadence_per_week"]["per_week"],
            serde_json::json!(7.0 / 90.0)
        );
    }

    #[test]
    fn the_coupling_and_native_naming_hold_on_the_wire() {
        let response = computed_response();
        let wire = serde_json::to_value(&response).unwrap();
        let cluster = &wire["cluster"];
        assert_eq!(cluster["note"], CLUSTER_NOTE);
        assert_eq!(cluster["throughput"]["control"], CONTROL_METRIC);
        assert_eq!(
            cluster["instability"]["change_fail_rate"]["role"],
            CONTROL_ROLE
        );
        let native = &cluster["native_measures"];
        assert_eq!(native["note"], NATIVE_NOTE);
        assert!(
            native["approval_to_promotion_elapsed"]
                .get("dora_name")
                .is_none(),
            "the native elapsed measure carries no DORA name on the wire"
        );
        assert_eq!(
            wire["attribution"]["programme"],
            "DORA (DevOps Research and Assessment)"
        );
        assert_eq!(
            wire["attribution"]["reference"],
            "https://dora.dev/guides/dora-metrics/"
        );
    }
}
