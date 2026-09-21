//! The interview repair mode. Repair is a MODE of the one interview state
//! machine — not a fork of it: the same `DI_*` conflict vocabulary, the
//! same revision CAS, and the same deterministic floor govern the repair
//! path exactly as they govern the answering path.
//!
//! Repair-policy vocabulary: `RepairPolicyMode`
//! (`pi_agent_rust/src/extensions.rs:2048`), the same vocabulary gajae's
//! `deep-interview-repair-cli.md` documents for the CLI repair flows. The
//! port-spec note of record lives beside the 1.27.32 Ask port spec (the
//! repair section); the kernel-side pin for this law is the docs-truth
//! guard `repair_is_mode_not_fork`.

pub fn repair_revision_conflict(expected: u64, actual: u64) -> String {
    format!(
        "DI_STATE_REVISION_CONFLICT expected {} actual {}",
        expected, actual
    )
}
