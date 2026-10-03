// The labelled measurement configuration, asserted from OUTSIDE the module.
//
// ## The three claims, and why they need an outside reader
//
// 1. **Production builds keep promotion disabled.** `PROMOTION_ENABLED` is a
//    compile-time constant in another module, and a measurement posture that could
//    be confused for production is exactly the failure D-C's first clause exists to
//    prevent. A module cannot assert its own posture's absence.
// 2. **The configuration is labelled.** A measurement build must be
//    distinguishable **at the artifact level**, so it can never be mistaken for
//    production by someone holding only the binary.
// 3. **The falsifier fires.** A planted divergent trace must reach the release
//    decision and be **refused with no state change** — driven through the real
//    replay path, not a hand-built report.
//
// ## Why no env var appears here
//
// If this posture were reachable by configuration, "promotion is disabled" would
// become a claim about deployment state rather than about the artifact. The pin
// below asserts the property directly: the production law is a constant, and the
// measurement posture is a parameter a caller must supply.

mod common;

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// **The production law.** `PROMOTION_ENABLED` is still a compile-time `false`.
///
/// Read from the source rather than imported, because the constant is
/// `pub(crate)` and this file's job is to assert the **text** a future round would
/// have to change. A round that flipped it has to edit this line, and the grep for
/// a `cfg` or env reach below fails in the same commit.
#[test]
fn production_builds_keep_promotion_disabled() {
    let src = common::code_only(&read("src/workflow/create/promote.rs"));
    let production = match src.find("#[cfg(test)]") {
        Some(idx) => &src[..idx],
        None => src.as_str(),
    };
    assert!(
        production.contains("const PROMOTION_ENABLED: bool = false"),
        "PROMOTION_ENABLED is no longer a compile-time false - read the law before editing it"
    );
}

/// No configuration or environment variable can reach the production switch.
///
/// This is the structural half of the law above. A `cfg!(feature = ...)` or an
/// `env::var` branch in the promotion module would make "disabled" a deployment
/// claim; the measurement posture exists precisely so it does not have to be.
#[test]
fn the_production_switch_has_no_configuration_reach() {
    let src = common::code_only(&read("src/workflow/create/promote.rs"));
    let production = match src.find("#[cfg(test)]") {
        Some(idx) => &src[..idx],
        None => src.as_str(),
    };
    for forbidden in ["env::var", "std::env", "cfg!(feature", "option_env!"] {
        assert!(
            !production.contains(forbidden),
            "the promotion module reads {forbidden}; the production law must not be a deployment claim"
        );
    }
}

/// The measurement configuration is labelled, and the label is conspicuous.
///
/// Asserted on the **enum's own spelling**, so a build carrying it says so in any
/// artefact it produces.
#[test]
fn the_measurement_configuration_is_labelled() {
    use brain_server::workflow::measurement::MeasurementConfiguration;

    assert_eq!(
        MeasurementConfiguration::Production.label(),
        "production",
        "the production posture must name itself plainly"
    );
    assert_eq!(
        MeasurementConfiguration::Measurement.label(),
        "measurement",
        "the measurement posture must name itself plainly"
    );
    // And the two are distinguishable, which is the whole point of a posture.
    assert_ne!(
        MeasurementConfiguration::Production.label(),
        MeasurementConfiguration::Measurement.label()
    );
    assert!(!MeasurementConfiguration::Production.is_labelled());
    assert!(MeasurementConfiguration::Measurement.is_labelled());
}

/// **The falsifier fires.** A planted divergent trace is refused, with no state
/// change, through the real replay path.
///
/// The fixture rewrites one `seq`, so no digest changes and the divergence is
/// **order-only** — the case a digest-only rule would miss. The plant helper
/// returns how many rows it changed and the pin asserts exactly one, so a plant
/// that matched nothing cannot leave this green.
#[test]
fn the_replay_falsifier_fires_on_a_planted_divergent_trace() {
    use brain_server::workflow::measurement::{
        MeasurementConfiguration, divergence_counts, falsify_verdict, open_measurement_run,
        plant_divergent_trace, replay_verdict_for,
    };

    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory");
    brain_server::migration::run_migration(&mut conn, 1).expect("schema");

    let run_id = open_measurement_run(&mut conn).expect("a delivery run opens");

    // Plant the divergence, and prove the plant landed.
    let changed = plant_divergent_trace(&conn, run_id).expect("the plant runs");
    assert_eq!(changed, 1, "the plant must rewrite exactly one ordinal");

    let (mismatched, compared) = divergence_counts(&conn, run_id).expect("the report assembles");
    assert!(
        mismatched > 0 && compared > 0,
        "the fixture must actually diverge ({mismatched}/{compared}), or the falsifier proves nothing"
    );

    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM delivery_traces", [], |r| r.get(0))
        .expect("countable");

    let verdict = replay_verdict_for(&conn, run_id).expect("the gate classifies");
    let outcome = falsify_verdict(MeasurementConfiguration::Measurement, "rel-1", verdict);
    assert!(
        outcome_is_refused(&outcome),
        "a divergent trace must be refused in the measurement posture, got {outcome:?}"
    );

    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM delivery_traces", [], |r| r.get(0))
        .expect("countable");
    assert_eq!(before, after, "the gate refuses; it never repairs");
}

/// Production refuses even a clean report — the law is checked first.
#[test]
fn production_refuses_a_clean_report_in_the_measurement_gates_absence() {
    use brain_server::workflow::measurement::{
        MeasurementConfiguration, divergence_counts, falsify_verdict, open_measurement_run,
        replay_verdict_for,
    };

    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory");
    brain_server::migration::run_migration(&mut conn, 1).expect("schema");

    let run_id = open_measurement_run(&mut conn).expect("a delivery run opens");
    let (_, compared) = divergence_counts(&conn, run_id).expect("the report assembles");
    assert!(compared > 0, "the fixture must compare something");

    let verdict = replay_verdict_for(&conn, run_id).expect("the gate classifies");
    let outcome = falsify_verdict(MeasurementConfiguration::Production, "rel-1", verdict);
    assert!(
        outcome_is_disabled(&outcome),
        "a clean report must not promote in production, got {outcome:?}"
    );
}

/// Every fired promotion carries a decision row naming the posture and the actor.
///
/// The R64′ law: a firing that left no attributable row did not happen.
#[test]
fn every_fired_promotion_is_attributable() {
    use brain_server::workflow::measurement::{
        MEASUREMENT_ACTOR, MeasurementConfiguration, falsify_verdict, firing_is_attributable,
        open_measurement_run, replay_verdict_for,
    };

    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mut conn = rusqlite::Connection::open_in_memory().expect("in-memory");
    brain_server::migration::run_migration(&mut conn, 1).expect("schema");

    let run_id = open_measurement_run(&mut conn).expect("a delivery run opens");
    let verdict = replay_verdict_for(&conn, run_id).expect("the gate classifies");
    let outcome = falsify_verdict(MeasurementConfiguration::Measurement, "rel-1", verdict);
    assert!(
        firing_is_attributable(&outcome),
        "a firing must be attributable to a labelled configuration"
    );
    assert_eq!(MEASUREMENT_ACTOR, "simulated-measurement");
}

fn outcome_is_refused(outcome: &brain_server::workflow::measurement::FalsificationOutcome) -> bool {
    matches!(
        outcome,
        brain_server::workflow::measurement::FalsificationOutcome::Refused {
            state_changed: false,
            ..
        }
    )
}

fn outcome_is_disabled(
    outcome: &brain_server::workflow::measurement::FalsificationOutcome,
) -> bool {
    matches!(
        outcome,
        brain_server::workflow::measurement::FalsificationOutcome::DisabledInProduction
    )
}
