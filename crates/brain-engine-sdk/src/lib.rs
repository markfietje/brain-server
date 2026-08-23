//! The engine ABI for the governed-workflow harness.
//!
//! Engines compile against this crate only: [`pure`] carries the deterministic
//! cores, [`policy`] the law/compliance vocabulary, [`host`] the
//! storage-agnostic `WorkflowHost` seam. The crate has zero dependencies and
//! forbids `unsafe`; every host signature is value-typed (`i64`/`&str`) so a
//! future Postgres (or any transactional) adapter implements the same trait
//! without an ABI break.
//!
//! Semver: a minor bump may add items; anything that removes or reshapes a
//! public item is a breaking release. Storage stays behind the trait — engines
//! never open a database.

#![forbid(unsafe_code)]
// Tests assert via unwrap/expect; production code keeps the deny.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

/// This crate's ABI version. Hosts and engines check compatibility with
/// [`requires_host`] before wiring.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether this SDK satisfies a host's minimum required version
/// (`"major.minor[.patch]"`). Compares component-wise; missing components
/// default to 0.
pub fn requires_host(min: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (have, need) = (parse(VERSION), parse(min));
    for i in 0..need.len().max(have.len()) {
        let h = have.get(i).copied().unwrap_or(0);
        let n = need.get(i).copied().unwrap_or(0);
        if h != n {
            return h > n;
        }
    }
    true
}

pub mod host;
pub mod policy;
pub mod pure;

// The plugin kernel + agent harness line. Opt-in: without the feature the
// crate compiles exactly as before (no kernel, no extra dependencies).
#[cfg(feature = "harness-kernel")]
pub mod capability;
#[cfg(feature = "harness-kernel")]
pub mod env;
#[cfg(feature = "harness-kernel")]
pub mod events;
#[cfg(feature = "harness-kernel")]
pub mod harness;
#[cfg(feature = "harness-kernel")]
pub mod hostcall;
#[cfg(feature = "harness-kernel")]
pub mod loader;
#[cfg(feature = "harness-kernel")]
pub mod plugin;
#[cfg(feature = "harness-kernel")]
pub mod prompt;
#[cfg(feature = "harness-kernel")]
pub mod session;
#[cfg(feature = "harness-kernel")]
pub mod tools;
#[cfg(feature = "harness-kernel")]
pub mod workflow;
#[cfg(feature = "harness-kernel")]
pub mod workflow_state;

/// The state-key routing contract (`Decision` + `decide`) — normative engine
/// ABI, re-exported at the crate root so engines never path into the module.
#[cfg(feature = "harness-kernel")]
pub use workflow_state::{Decision, decide};

/// The scoreboard surface (`sdk::scoreboard::build`) — a re-export of the
/// pure scorer, never a second implementation.
pub mod scoreboard {
    pub use crate::pure::qa_score::{RunArtifacts, Scoreboard, StepRow, scoreboard as build};
}

/// The calibration surface: scorer-vs-human agreement and its cadence gates.
pub mod calibration {
    pub use crate::pure::calibration::{
        CalibrationRecord, NO_KAPPA, SCORER_VERSION as CALIBRATION_SCORER_VERSION,
        WEEK_SECS as CALIBRATION_WEEK_SECS, kappa_units, month_due, month_index, week_due,
    };
}
#[cfg(feature = "harness-kernel")]
pub mod trust;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_cargo_and_gate() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert!(requires_host("1.28"));
        assert!(requires_host("1.28.3"));
        assert!(requires_host("1.27.9"), "patch-lower hosts are accepted");
        assert!(!requires_host("1.29"));
        assert!(!requires_host("2.0"));
        assert!(requires_host("1"), "prefix requirements pad to zero");
    }
}
