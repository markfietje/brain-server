//! The decision-harness kernel seam — the kernel's face over the SDK's
//! always-on [`brain_engine_sdk::decision`] module.
//!
//! Authority law (monotonic-narrow), verbatim from the authoritative
//! architecture: **a DecisionModel proposes; only the gate disposes.**
//! A model's output alone can never mutate durable state: the trait's
//! receivers are `&self`, the context is read-only, and every seam type
//! is plain data. Promotion, persistence, routes, and side effects are
//! the pipeline's and the gate's job — this module ships NONE of them.
//!
//! This round is the SEAM only: the SDK trait + types consumed
//! ([`model`]), the deterministic reference model over a declared rule
//! table and the decide adapter ([`models`]), and the additive
//! session-log kind constants the decision-run record (the next lane's
//! consumer) will read. No routes, no state change, no GDL contact —
//! inert to the loop (the non-troubleshoot law): nothing outside this
//! module reads these constants, and no gate consults them.

pub(crate) mod model;
pub(crate) mod models;

/// Additive session-log kinds for the decision-run record (its consumer
/// lands with the run-record lane; recorded per run from day one so
/// nothing exploratory can masquerade as governed). Constants ONLY this
/// round: zero migration, zero routes, inert to the loop.
pub(crate) const DECISION_RUN_KIND: &str = "decision_run";
pub(crate) const DECISION_OUTPUT_KIND: &str = "decision_output";
pub(crate) const DECISION_REFUSAL_KIND: &str = "decision_refusal";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::session_log;
    use brain_engine_sdk::decision::RunMode;
    use rusqlite::Connection;

    /// The additive kind constants are distinct, carry no reserved
    /// `control:` prefix, round-trip through the REAL session-log
    /// append/read-back (they are representable), and `RunMode` carries
    /// its closed string form — while staying inert: no projection treats
    /// them as control rows and nothing outside this module reads them
    /// (the compiler enforces the reader side; this test pins the rest).
    #[test]
    fn run_mode_constants_are_additive_and_inert() {
        let kinds = [
            DECISION_RUN_KIND,
            DECISION_OUTPUT_KIND,
            DECISION_REFUSAL_KIND,
        ];
        for kind in kinds {
            assert!(!kind.is_empty(), "kinds are non-empty");
            assert!(
                !kind.starts_with("control:"),
                "the control: family stays reserved: {kind}"
            );
        }
        assert_ne!(kinds[0], kinds[1]);
        assert_ne!(kinds[1], kinds[2]);
        assert_ne!(kinds[0], kinds[2]);

        assert_eq!(RunMode::Deterministic.as_str(), "deterministic");
        assert_eq!(RunMode::Exploratory.as_str(), "exploratory");

        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        let payload = format!(
            r#"{{"mode":"{}","model_id":"rules-reference"}}"#,
            RunMode::Deterministic.as_str()
        );
        let (created, seq) =
            session_log::append(&conn, 1, DECISION_RUN_KIND, &payload, "decision:1", 1).unwrap();
        assert!(created);
        let replayed = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let row = replayed
            .iter()
            .find(|r| r.kind == DECISION_RUN_KIND)
            .unwrap_or_else(|| panic!("the appended decision row must read back"));
        assert_eq!(row.seq, seq);
        assert_eq!(row.payload_json, payload);
        // Inert: the row is an ordinary event — same seq space, no
        // control-family treatment, no gate consumed it on the way through.
    }
}
