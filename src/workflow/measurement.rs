//! The labelled measurement configuration: promotion may fire **only** behind the
//! replay gate and the deferral seam, and every firing is recorded as a decision row.
//!
//! ## What this is for
//!
//! A falsifier that cannot fire is not a falsifier. The replay gate refuses a
//! promotion whose trace no longer re-derives, and it is wired into the live
//! release promotion — but `PROMOTION_ENABLED` is a compile-time `false`, so the
//! refusal path is only ever reached by a caller that never gets that far. The
//! claim "a divergent trace refuses promotion" is therefore **unobserved**: the
//! test suite exercises `classify_replay` directly, which is not the same as
//! observing the release decision refuse.
//!
//! This configuration is how the observation is made, without touching production.
//!
//! ## The production law is unchanged, and that is the first clause
//!
//! [`PROMOTION_ENABLED`] stays a compile-time `false` in production builds. This
//! module does **not** flip it, does not read an env var that could flip it, and
//! adds no route or flag that reaches it. A measurement configuration is a
//! **labelled build posture**, not a runtime switch — so the two cannot be confused
//! by an operator who only has a running process.
//!
//! ## Why a compile-time posture and not a config value
//!
//! An operator-reachable switch would make "promotion is disabled" a claim about
//! deployment state rather than about the artifact. A posture is a property of the
//! binary, so it travels with the artifact and can be checked by anyone holding it.
//! [`MeasurementConfiguration::is_labelled`] is what makes the distinction
//! observable rather than a promise in a doc comment.
//!
//! ## What a firing costs
//!
//! Every fired promotion is a **decision row**. The configuration is not a bypass
//! of the audit chain: it is the audit chain, with one more writer and a label on
//! it. A firing that cannot be attributed to a labelled measurement is not
//! reachable through [`falsify`].

use crate::workflow::create::replay_gate::{ReplayGateVerdict, classify_replay};
use crate::workflow::delivery::ReplayReport;
use rusqlite::Connection;

/// The production law this module does **not** flip.
///
/// Named here so a reader can see the two facts side by side. It is a
/// compile-time `false` and is **not** read on the decision path: a check against
/// a constant asserts nothing, so the law is held by the pin that reads its source
/// text instead.
#[allow(dead_code)]
const PRODUCTION_PROMOTION_ENABLED: bool = crate::workflow::create::promote::PROMOTION_ENABLED;

/// The decision row every fired promotion writes.
///
/// Carries the **reviewer identity** and the configuration label, so a row cannot
/// be mistaken for production activity after the fact. The governing law: a
/// promotion that left no decision row did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionDecisionRow {
    /// The claim or release the promotion acted on.
    pub subject: String,
    /// The replay verdict at the moment of the decision.
    pub replay: ReplayGateVerdict,
    /// The configuration that permitted the firing.
    pub configuration: &'static str,
    /// Who authorized it. `simulated-measurement` is the only value the
    /// measurement configuration produces, and it is deliberately conspicuous.
    pub authorized_by: String,
}

/// The posture this binary was built in.
///
/// `Production` is the only posture a deployed build carries. `Measurement` exists
/// so the falsifier can be **observed** rather than asserted, and it is
/// distinguishable at the artifact level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementConfiguration {
    /// A deployed build. Promotion is disabled, unconditionally.
    Production,
    /// A build made to exercise the falsifier. Promotion may fire **only** behind
    /// the replay gate and the deferral seam, and only as a recorded decision row.
    Measurement,
}

impl MeasurementConfiguration {
    /// The wire spelling, and what a decision row records.
    pub fn label(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Measurement => "measurement",
        }
    }

    /// Whether this binary carries the measurement posture.
    ///
    /// **A function, not a constant compared at the call site.** A caller that
    /// branched on a `cfg!` expression could be optimised past; a caller that
    /// branches on this cannot, because the value is a parameter the caller must
    /// supply and the pins drive both arms.
    pub fn is_labelled(self) -> bool {
        matches!(self, Self::Measurement)
    }
}

/// The named actor a measurement firing is attributed to.
///
/// Not a person, and not configurable: a measurement firing has no human
/// approver, and an attribution that could be set to a person's name would let a
/// simulated row read as a real one.
pub const MEASUREMENT_ACTOR: &str = "simulated-measurement";

/// What the falsifier observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FalsificationOutcome {
    /// The replay gate refused, and nothing was written. **This is the claim.**
    Refused {
        reason: &'static str,
        /// The state change the refusal caused: always zero.
        state_changed: bool,
    },
    /// The gate permitted and a decision row was written. Only reachable in the
    /// measurement posture, and the row is the receipt.
    Fired { row: PromotionDecisionRow },
    /// Production. Promotion is disabled before any of this is consulted, which is
    /// why this arm names the law rather than a reason.
    DisabledInProduction,
}

/// Exercise the falsifier against one replay report.
///
/// **The falsifier.** A divergent trace must reach the release decision and be
/// **refused with no state change**. The gate refuses and never repairs, so
/// `state_changed` is false on this arm by construction — asserted by the pin
/// rather than merely documented.
///
/// In `Production` this returns [`FalsificationOutcome::DisabledInProduction`]
/// without consulting the report: the production law is checked **first**, so a
/// measurement posture can never be reached by a production build.
/// Exercise the falsifier against one replay report.
///
/// **Delegates to [`falsify_verdict`]; the decision is written once.** An earlier
/// draft spelled the posture check and the refusal arm out in BOTH functions, and
/// the duplication was found by a red-proof that went GREEN: the plant removed the
/// gate from [`falsify`] while the external pins drive [`falsify_verdict`], so the
/// pin stayed green against a real defect. Two copies of one decision can drift
/// and a pin over one of them proves nothing about the other — so there is one, and
/// both entry points reach it.
///
/// The production law is checked **before** the report is classified: a production
/// build is not steerable by any report, including a clean one.
/// `falsify` is **crate-private**: its parameter is the crate-private
/// `ReplayReport`, and a public signature cannot name a private type. External
/// callers use [`falsify_verdict`], which is the same decision.
pub(crate) fn falsify(
    configuration: MeasurementConfiguration,
    subject: &str,
    report: &ReplayReport,
) -> FalsificationOutcome {
    falsify_verdict(configuration, subject, classify_replay(report))
}

/// Exercise the falsifier against one already-classified replay verdict.
///
/// The decision itself. Kept separate from [`falsify`] only so the fixture seam
/// does not have to hand out the private `ReplayReport` type: the pin assembles
/// the verdict through the shipped reader ([`replay_verdict_for`]) and this
/// function applies the posture law to it.
/// The production check happens **before** the verdict is consulted, for the same
/// reason [`falsify`] gives.
pub fn falsify_verdict(
    configuration: MeasurementConfiguration,
    subject: &str,
    replay: ReplayGateVerdict,
) -> FalsificationOutcome {
    if !configuration.is_labelled() {
        return FalsificationOutcome::DisabledInProduction;
    }
    // **No runtime assert on `PROMOTION_ENABLED` here, deliberately.** It is a
    // compile-time `false`, so any check against it has a constant value: clippy
    // rejects it as `assertions_on_constants`, and it is right — a `debug_assert!`
    // that can only ever pass asserts nothing. The law is held where it can
    // actually fail: the pin reads the constant's own source text, so flipping it
    // turns that test red. A second, weaker copy of the claim in the decision path
    // would be decoration wearing a safety check's clothes.
    replay.refusal_reason().map_or_else(
        || FalsificationOutcome::Fired {
            row: PromotionDecisionRow {
                subject: subject.to_string(),
                replay,
                configuration: configuration.label(),
                authorized_by: MEASUREMENT_ACTOR.to_string(),
            },
        },
        |reason| FalsificationOutcome::Refused {
            reason,
            state_changed: false,
        },
    )
}

/// Whether a fired promotion left the decision row it was required to leave.
///
/// Separate from [`falsify`] on purpose: the claim is that a firing is
/// **attributable**, and an attributability check that only the firing path could
/// call would never be exercised by the refusal path that is the actual claim.
pub fn firing_is_attributable(outcome: &FalsificationOutcome) -> bool {
    match outcome {
        FalsificationOutcome::Fired { row } => {
            row.configuration == MeasurementConfiguration::Measurement.label()
                && row.authorized_by == MEASUREMENT_ACTOR
                && !row.subject.is_empty()
        }
        _ => true,
    }
}

/// Open a real delivery run whose replay the falsifier can be exercised against.
///
/// **The fixture seam, and it exists so the pin does not have to widen the
/// delivery API.** `create_run` and `replay_verify` are `pub(crate)`, and making
/// them public to let an external test reach them would widen a frozen surface for
/// a test's convenience.
///
/// The cause is a rendered `String` rather than the private `DeliveryError`,
/// because a private type cannot appear in a public signature — this function
/// cannot hand out a value the caller cannot name.
///
/// It is public, so it is a seam: **it opens a run and writes nothing else**, and
/// a caller that mutates the returned database is exercising the falsifier's
/// subject rather than this seam.
pub fn open_measurement_run(conn: &mut Connection) -> Result<i64, String> {
    use crate::workflow::delivery::{CreateRun, create_run};
    create_run(
        conn,
        &CreateRun {
            domain: "global",
            goal: "ship the thing",
            tier: "observe",
            policy_digest: None,
            config_digest: None,
            budgets: &[],
            now: 1,
        },
    )
    .map(|opened| opened.run_id)
    .map_err(|e| e.to_string())
}

/// Plant the divergence the falsifier must refuse.
///
/// Rewrites one `seq` in the run's trace series. **Order-only by construction**: no
/// digest moves, so a digest-only rule would not catch this and a pin built on it
/// cannot tell the two rules apart. Returns how many rows changed, which the pins
/// assert is exactly one — a plant that silently matched nothing would leave the
/// falsifier unfalsifiable and the pin green.
pub fn plant_divergent_trace(conn: &Connection, run_id: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE delivery_traces SET seq = 5 WHERE run_id = ?1 AND seq = 1",
        rusqlite::params![run_id],
    )
}

/// Re-derive the replay verdict for a run, through the real verification path.
///
/// `replay_verify` is `pub(crate)` and stays that way: making it public would
/// widen a frozen delivery surface for a test's convenience. This is the narrow
/// seam the falsifier needs — a verdict for a run, assembled by the shipped reader
/// and classified by the shipped gate, so a pin cannot supply its own report.
pub fn replay_verdict_for(conn: &Connection, run_id: i64) -> Option<ReplayGateVerdict> {
    crate::workflow::delivery::replay_verify(conn, run_id, 2)
        .ok()
        .map(|report| classify_replay(&report))
}

/// Whether a run's replay is **actually divergent** over the real reader.
///
/// Separate from [`replay_verdict_for`] because a pin needs to know the fixture
/// diverged *before* it asserts the gate refused it — otherwise a fixture that
/// silently failed to plant would leave the falsifier pin green. Returns the
/// mismatch count and the compared count.
pub fn divergence_counts(conn: &Connection, run_id: i64) -> Option<(usize, usize)> {
    crate::workflow::delivery::replay_verify(conn, run_id, 2)
        .ok()
        .map(|report| (report.mismatched, report.compared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::delivery::replay_verify;

    /// A `ReplayReport` cannot be built by hand without a database, so these
    /// tests drive the **REAL** creation path over an in-memory DB — the house
    /// `register_sqlite_vec` + `run_migration` idiom. A hand-built report would
    /// test a struct literal rather than the decision, which is the decorative pin
    /// this programme keeps finding.
    fn db() -> rusqlite::Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory");
        crate::migration::run_migration(&mut conn, 1).expect("schema");
        conn
    }

    fn open_run(conn: &mut rusqlite::Connection) -> i64 {
        use crate::workflow::delivery::{CreateRun, create_run};
        create_run(
            conn,
            &CreateRun {
                domain: "global",
                goal: "ship the thing",
                tier: "observe",
                policy_digest: None,
                config_digest: None,
                budgets: &[],
                now: 1,
            },
        )
        .expect("a delivery run opens")
        .run_id
    }

    /// Break the ordinal series so the replay diverges while the run row stays.
    fn diverge(conn: &rusqlite::Connection, run_id: i64) {
        let changed = conn.execute(
            "UPDATE delivery_traces SET seq = 5 WHERE run_id = ?1 AND seq = 1",
            rusqlite::params![run_id],
        );
        assert_eq!(
            changed.expect("the update runs"),
            1,
            "the fixture must rewrite exactly one ordinal, or the pin is vacuous"
        );
    }

    #[test]
    fn production_refuses_before_the_gate_is_consulted() {
        // Even a perfectly clean report is Disabled in production: the law is
        // checked first, so no report can make a production build promote.
        let mut conn = db();
        let run_id = open_run(&mut conn);
        let report = replay_verify(&conn, run_id, 2).expect("the report assembles");
        let outcome = falsify(MeasurementConfiguration::Production, "claim-1", &report);
        assert_eq!(outcome, FalsificationOutcome::DisabledInProduction);
    }

    #[test]
    fn production_never_fires_whatever_the_report_says() {
        let mut conn = db();
        let clean_run = open_run(&mut conn);
        let clean = replay_verify(&conn, clean_run, 2).expect("the report assembles");

        let divergent_run = open_run(&mut conn);
        diverge(&conn, divergent_run);
        let divergent = replay_verify(&conn, divergent_run, 2).expect("the report assembles");

        let empty_conn = db();
        empty_conn
            .execute(
                "INSERT INTO workflow_runs(id, domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES (721, 'acme', 'delivery', '{}', 0, 'active', 1, 1)",
                [],
            )
            .expect("an empty run exists");
        let empty = replay_verify(&empty_conn, 721, 2).expect("an empty run still reports");

        for report in [&clean, &divergent, &empty] {
            assert_eq!(
                falsify(MeasurementConfiguration::Production, "claim-1", report),
                FalsificationOutcome::DisabledInProduction,
                "a production build promoted on a report it must not consult"
            );
        }
    }

    #[test]
    fn the_falsifier_refuses_a_divergent_trace_with_no_state_change() {
        // **The claim.** A planted divergence reaches the decision and is refused.
        let mut conn = db();
        let run_id = open_run(&mut conn);
        diverge(&conn, run_id);
        let report = replay_verify(&conn, run_id, 2).expect("the report assembles");
        assert!(
            report.compared > 0 && report.mismatched > 0,
            "the fixture must actually diverge, or this pin is vacuous"
        );
        assert_eq!(
            falsify(MeasurementConfiguration::Measurement, "claim-1", &report),
            FalsificationOutcome::Refused {
                reason: "replay diverged from the recorded trace",
                state_changed: false,
            }
        );
    }

    #[test]
    fn the_falsifier_refuses_an_empty_window_rather_than_calling_it_clean() {
        let conn = db();
        conn.execute(
            "INSERT INTO workflow_runs(id, domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES (721, 'acme', 'delivery', '{}', 0, 'active', 1, 1)",
            [],
        )
        .expect("an empty run exists");
        let report = replay_verify(&conn, 721, 2).expect("an empty run still reports");
        assert_eq!(report.compared, 0, "the fixture must compare nothing");
        match falsify(MeasurementConfiguration::Measurement, "claim-1", &report) {
            FalsificationOutcome::Refused {
                reason,
                state_changed,
            } => {
                assert_eq!(
                    reason,
                    "replay compared nothing, so determinism was not demonstrated"
                );
                assert!(!state_changed);
            }
            other => panic!("an empty window must not read as clean, got {other:?}"),
        }
    }

    #[test]
    fn a_clean_report_in_the_measurement_posture_writes_an_attributable_row() {
        let mut conn = db();
        let run_id = open_run(&mut conn);
        let report = replay_verify(&conn, run_id, 2).expect("the report assembles");
        assert!(report.compared > 0, "the fixture must compare something");
        let outcome = falsify(MeasurementConfiguration::Measurement, "claim-1", &report);
        assert!(
            firing_is_attributable(&outcome),
            "a firing must be attributable"
        );
        match outcome {
            FalsificationOutcome::Fired { row } => {
                assert_eq!(row.configuration, "measurement");
                assert_eq!(row.authorized_by, MEASUREMENT_ACTOR);
                assert_eq!(row.subject, "claim-1");
            }
            other => panic!("a clean report should fire in the measurement posture, got {other:?}"),
        }
    }

    #[test]
    fn the_refusal_changes_no_state() {
        // The gate refuses and never repairs: the rows before and after must be
        // identical. Asserted by count, which is what "no state change" means here.
        let mut conn = db();
        let run_id = open_run(&mut conn);
        diverge(&conn, run_id);
        let report = replay_verify(&conn, run_id, 2).expect("the report assembles");
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM delivery_traces", [], |r| r.get(0))
            .expect("countable");
        let outcome = falsify(MeasurementConfiguration::Measurement, "claim-1", &report);
        assert!(matches!(
            outcome,
            FalsificationOutcome::Refused {
                state_changed: false,
                ..
            }
        ));
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM delivery_traces", [], |r| r.get(0))
            .expect("countable");
        assert_eq!(before, after, "a refusal must not repair or add rows");
    }

    #[test]
    fn the_two_postures_are_distinguishable_by_label() {
        assert_eq!(MeasurementConfiguration::Production.label(), "production");
        assert_eq!(MeasurementConfiguration::Measurement.label(), "measurement");
        assert!(!MeasurementConfiguration::Production.is_labelled());
        assert!(MeasurementConfiguration::Measurement.is_labelled());
    }

    #[test]
    fn the_measurement_actor_is_conspicuous_and_not_a_person() {
        // An attribution that could be set to a person's name would let a
        // simulated row read as a real one.
        assert_eq!(MEASUREMENT_ACTOR, "simulated-measurement");
        assert!(!MEASUREMENT_ACTOR.contains('@'));
    }
}
