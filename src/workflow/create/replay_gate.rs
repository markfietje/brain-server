//! The replay divergence gate: the pure decision that turns a replay REPORT
//! into a refusal.
//!
//! ## What already exists, and what was missing
//!
//! The hard half of this round shipped before the round ran. The comparator
//! [`brain_delivery_core::compare_replay`] is in the delivery-core crate, and
//! [`crate::workflow::delivery::replay_verify`] assembles a [`ReplayReport`]
//! from a run's stored trace rows. Both are pinned, and the anti-vacuity device
//! is a golden vector — because a pin asserting only "no diffs" would pass on a
//! projection that mapped every field to a constant.
//!
//! **What did not exist is a gate.** `replay_verify` is reachable only from
//! `GET /workflow/runs/{id}/replay-verify`; it returns data and nothing refuses
//! on it. This module is that refusal, as a **pure decision**.
//!
//! ## Three properties, each structural
//!
//! 1. **An empty window is not a pass.** [`ReplayGateVerdict::InsufficientEvidence`]
//!    is a *distinct* outcome from [`ReplayGateVerdict::Clean`]. Zero compared
//!    positions cannot demonstrate determinism, and a gate that reads "no
//!    mismatches" on an empty comparison is the same defect as a validator that
//!    reports green having checked nothing.
//!
//! 2. **The gate refuses; it never repairs.** There is no write path here at
//!    all — no connection, no transaction, no statement. A gate that patched the
//!    divergence it was built to detect is how a specification stops being about
//!    the task, which is the revision-routing law the release plan states: the
//!    gate routes, it does not patch.
//!
//! 3. **Divergence is the crate's comparison widened by ordinal violations.**
//!    [`ReplayReport::is_clean`] already documents that it is **stricter** than
//!    `ReplayDiff::all_digests_match()`, because order violations are folded
//!    into `mismatched`. This module uses the stricter rule, and pins the
//!    difference so it cannot be silently relaxed to the looser one.
//!
//! ## What this is NOT
//!
//! ~~**No production consumer exists.**~~ **Superseded at R2 (2026-10-02).**
//! Promotion of a *claim* is still refused by a compile-time `false`
//! ([`crate::workflow::create::PROMOTION_ENABLED`]) by deliberate design — but
//! that is the **wrong seam**, and wiring a gate there would have produced a
//! green test suite over a function no caller can reach. The consumer landed on
//! the **live** release promotion instead: [`crate::workflow::releases::promote_release`]
//! reads this verdict after its `chain_defect` check and before
//! `brain_delivery_core::promote`, and a refusing verdict denies with no state
//! change. That path promotes for real, so the gate there is falsifiable — which
//! is the property the inert claim promotion could never have had.
//!
//! **The claim is narrow and the round says so.** This gate proves that a
//! promotion cannot rest on a trace that no longer re-derives. It says nothing
//! about classifier fidelity (a different axis entirely — `classify_replay`
//! never calls the classifier) and nothing about how *often* divergence occurs;
//! no out-of-sample figure is claimed.

use crate::workflow::delivery::ReplayReport;

/// What the gate decided about a replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayGateVerdict {
    /// The replay diverged from the recorded trace. **Refuses.**
    Divergent { compared: usize, mismatched: usize },
    /// The replay reproduced the recorded trace exactly.
    ///
    /// Reachable only with `compared > 0` — an empty window is never clean.
    Clean { compared: usize },
    /// Too little was compared to decide. **Refuses**, distinctly from
    /// divergence, so "we did not look" is never read as "we looked and it was
    /// fine".
    InsufficientEvidence { compared: usize },
}

impl ReplayGateVerdict {
    /// The two refusing verdicts' wire slugs, DISTINCT.
    ///
    /// A separate method from [`Self::refusal_reason`] because the two answer
    /// different questions: the reason is a sentence for a human reading an
    /// audit row, and this is a stable token for a caller that must branch or
    /// aggregate. Collapsing them would make `insufficient_evidence` — an empty
    /// window, where determinism was never demonstrated — indistinguishable from
    /// a real divergence at the call site, which is precisely the distinction
    /// this enum exists to keep.
    pub const fn refusal_code(self) -> &'static str {
        match self {
            ReplayGateVerdict::Divergent { .. } => "divergent",
            ReplayGateVerdict::InsufficientEvidence { .. } => "insufficient_evidence",
            ReplayGateVerdict::Clean { .. } => "clean",
        }
    }

    /// Whether this verdict permits a promotion.
    ///
    /// **Only [`Self::Clean`] does.** Divergence and insufficient evidence both
    /// refuse, and they refuse for different reasons — which is the point of
    /// keeping them apart.
    pub fn permits_promotion(self) -> bool {
        matches!(self, ReplayGateVerdict::Clean { .. })
    }

    /// Whether this verdict refuses, and why in one line.
    pub fn refusal_reason(self) -> Option<&'static str> {
        match self {
            ReplayGateVerdict::Divergent { .. } => Some("replay diverged from the recorded trace"),
            ReplayGateVerdict::InsufficientEvidence { .. } => {
                Some("replay compared nothing, so determinism was not demonstrated")
            }
            ReplayGateVerdict::Clean { .. } => None,
        }
    }
}

/// Classify a replay report into a promotion verdict.
///
/// Pure: reads the report, writes nothing, calls nothing, cannot fail. The
/// ordering is deliberate — **insufficient evidence is tested first**, so an
/// empty window can never fall through to the clean arm on the strength of a
/// `mismatched == 0` it earned by comparing nothing.
pub fn classify_replay(report: &ReplayReport) -> ReplayGateVerdict {
    // (1) Nothing was compared. This is NOT a pass, and it is checked before
    // the mismatch test so no zero-mismatch reading of an empty window can.
    if report.compared == 0 {
        return ReplayGateVerdict::InsufficientEvidence { compared: 0 };
    }
    // (2) Any divergence — digest mismatch OR ordinal violation, because
    // `is_clean` folds the latter into `mismatched` — refuses.
    if report.mismatched > 0 {
        return ReplayGateVerdict::Divergent {
            compared: report.compared,
            mismatched: report.mismatched,
        };
    }
    ReplayGateVerdict::Clean {
        compared: report.compared,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::delivery::replay_verify;

    /// A `ReplayReport` cannot be built by hand without a database, so these
    /// tests use the REAL creation path over an in-memory DB — the house
    /// `register_sqlite_vec` + `run_migration` idiom. A hand-built report would
    /// test the struct literal rather than the decision, which is the decorative
    /// pin this programme keeps finding.
    fn db() -> rusqlite::Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migration::run_migration(&mut conn, 1).unwrap();
        conn
    }

    /// Open a real delivery run. `create_run` writes the run row AND its first
    /// trace row, so the replay has something to compare — which is exactly the
    /// condition a gate must distinguish from an empty window.
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

    /// Break the ordinal series by inserting a SECOND trace row claiming an
    /// already-used `seq`. The digests stay contiguous and re-derivable, so the
    /// crate's own comparator still sees zero mismatches — which is the whole
    /// point: this is the case where `is_clean` and `all_digests_match` differ.
    fn break_ordinal_series(conn: &rusqlite::Connection, run_id: i64) {
        // `create_run` writes ONE row at seq 1, so the series 1..1 is contiguous.
        // REWRITING that row's `seq` to 5 makes the stored ordinal series 5..5,
        // which `read_run_traces` expects to be 1 -- an ORDER-ONLY violation.
        //
        // **Rewriting `seq` rather than inserting a row is the whole point.**
        // An inserted row re-derives a different digest, so it carries
        // `output_digest_differs` TOO, and a digest-only rule would refuse it as
        // well -- which would make this pin unable to tell the two rules apart.
        // The first version of this fixture inserted a row, and its red-proof
        // passed against the LOOSER rule, which is how the false green was
        // caught. Only a `seq` that changes the ordinal without changing any
        // digest isolates the rule actually under test.
        let changed = conn.execute(
            "UPDATE delivery_traces SET seq = 5 WHERE run_id = ?1 AND seq = 1",
            rusqlite::params![run_id],
        );
        assert_eq!(
            changed.unwrap(),
            1,
            "the fixture must rewrite exactly one ordinal, or the pin is vacuous"
        );
    }

    /// R58a.2 — a clean replay permits promotion.
    #[test]
    fn a_clean_replay_permits_promotion() {
        let mut conn = db();
        let run_id = open_run(&mut conn);
        let report = replay_verify(&conn, run_id, 2).expect("the report assembles");
        assert!(
            report.compared > 0,
            "the fixture must actually compare something, or this pin is vacuous"
        );
        assert_eq!(
            classify_replay(&report),
            ReplayGateVerdict::Clean {
                compared: report.compared
            }
        );
        assert!(classify_replay(&report).permits_promotion());
        assert_eq!(classify_replay(&report).refusal_reason(), None);
    }

    /// R58a.1 — an EMPTY window is insufficient evidence, never clean. This is
    /// the load-bearing refusal: a gate that reads "no mismatches" on nothing
    /// is a gate that passes by not looking.
    #[test]
    fn an_empty_window_is_insufficient_evidence_never_clean() {
        let conn = db();
        // A run row with NO trace rows, so the replay window is genuinely empty.
        conn.execute(
            "INSERT INTO workflow_runs(id, domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES (721, 'acme', 'delivery', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        let report = replay_verify(&conn, 721, 2).expect("an empty run still reports");
        assert_eq!(
            report.compared, 0,
            "the fixture must actually compare nothing, or this pin is vacuous"
        );
        let verdict = classify_replay(&report);
        assert_eq!(
            verdict,
            ReplayGateVerdict::InsufficientEvidence { compared: 0 }
        );
        assert!(
            !verdict.permits_promotion(),
            "comparing nothing must not promote"
        );
        assert!(
            verdict
                .refusal_reason()
                .is_some_and(|r| r.contains("compared nothing")),
            "the refusal must name the reason: {verdict:?}"
        );
    }

    /// R58a.3 — an ordinal violation refuses even with matching digests.
    ///
    /// This is the `is_clean` vs `all_digests_match` difference: the crate's
    /// comparator sees `mismatched == 0` over ITS count, while this report folds
    /// the broken ordinal series in. Pinning it stops the stricter rule being
    /// quietly relaxed to the looser one.
    #[test]
    fn an_ordinal_violation_refuses_even_with_matching_digests() {
        let mut conn = db();
        let run_id = open_run(&mut conn);
        break_ordinal_series(&conn, run_id);
        let report = replay_verify(&conn, run_id, 2).expect("the report assembles");
        assert!(
            !report.order_ok,
            "the fixture must actually break the ordinal series"
        );
        assert!(
            report.mismatched > 0,
            "the order violation must be folded into the count this gate reads, \
             or the stricter rule is not in force"
        );
        assert!(
            !report.order_ok,
            "the fixture must break the ordinal series"
        );
        assert!(
            report.mismatched > 0,
            "the order violation must be folded into the count this gate reads, \
             or the stricter rule is not in force"
        );
        // **This pin is only meaningful if the divergence is ORDER-ONLY.**
        // The fixture rewrites a `seq`, so no digest changes: an inserted row
        // would re-derive a different digest and carry an output-digest label
        // too, and a digest-only rule would refuse it as well -- leaving this
        // pin unable to tell which rule is in force. The first version of this
        // test inserted a row and its red-proof PASSED against the looser rule,
        // which is how the false green was caught.
        let verdict = classify_replay(&report);
        assert_eq!(
            verdict,
            ReplayGateVerdict::Divergent {
                compared: report.compared,
                mismatched: report.mismatched,
            }
        );
        assert!(!verdict.permits_promotion());
    }

    /// R58a.4 / R58a.5 — the gate is pure and non-repairing: classifying a
    /// divergent report must leave every stored row byte-identical. A gate that
    /// rewrote what it detected would be patching the specification, which the
    /// revision-routing law forbids.
    #[test]
    fn the_gate_never_writes_and_never_repairs() {
        let mut conn = db();
        let run_id = open_run(&mut conn);
        break_ordinal_series(&conn, run_id);
        let before = trace_fingerprint(&conn, run_id);
        let report = replay_verify(&conn, run_id, 2).expect("the report assembles");
        assert!(!classify_replay(&report).permits_promotion());
        let after = trace_fingerprint(&conn, run_id);
        assert_eq!(
            before, after,
            "classifying a divergent report must not touch a single stored row"
        );
    }

    /// A fingerprint over the run's trace rows, so "nothing was written" is
    /// measured rather than assumed.
    fn trace_fingerprint(conn: &rusqlite::Connection, run_id: i64) -> String {
        let mut stmt = conn
            .prepare(
                "SELECT id, seq, stage, phase, status, COALESCE(model_ref,'') \
                 FROM delivery_traces WHERE run_id = ?1 ORDER BY id",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![run_id], |r| {
                Ok(format!(
                    "{}|{}|{}|{}|{}|{}",
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .unwrap();
        rows.flatten().collect::<Vec<_>>().join(";")
    }

    /// The three outcomes are mutually exclusive and cover every input — the
    /// report is a total decision, which is what makes it usable as a gate.
    #[test]
    fn the_three_verdicts_are_exhaustive_and_distinct() {
        let mut conn = db();
        // Clean: a real run, nothing tampered.
        let clean_run = open_run(&mut conn);
        let clean = classify_replay(&replay_verify(&conn, clean_run, 2).unwrap());
        // Insufficient: a run row with no trace rows.
        conn.execute(
            "INSERT INTO workflow_runs(id, domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES (731, 'acme', 'delivery', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        let empty = classify_replay(&replay_verify(&conn, 731, 2).unwrap());
        // Divergent: a real run whose ordinal series is broken.
        let divergent_run = open_run(&mut conn);
        break_ordinal_series(&conn, divergent_run);
        let divergent = classify_replay(&replay_verify(&conn, divergent_run, 2).unwrap());

        assert_ne!(clean, empty);
        assert_ne!(clean, divergent);
        assert_ne!(empty, divergent);
        // Exactly one permits promotion, and it is the clean one.
        assert_eq!(
            [clean, empty, divergent]
                .iter()
                .filter(|v| v.permits_promotion())
                .count(),
            1
        );
        // Both refusals carry a reason, and the reasons DIFFER — "we did not
        // look" and "we looked and it diverged" must never read alike.
        assert!(empty.refusal_reason().is_some());
        assert!(divergent.refusal_reason().is_some());
        assert_ne!(empty.refusal_reason(), divergent.refusal_reason());
    }
}
