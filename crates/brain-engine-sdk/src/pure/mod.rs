//! Deterministic cores: pure functions with no I/O, no clock, no host calls.
//!
//! These are oracle-pinned (not mathematically closed); the tests shipped here
//! are the contract every engine inherits. Honest ceiling: `reduce` can never
//! be PROVEN false-merge-free — a reducer that merges near-identical claims is
//! one that compounds errors, so the guard stays conservative.

pub mod aftersales;
pub mod calibration;
pub mod complaint;
pub mod consent;
pub mod evidence;
pub mod handoff;
pub mod kcs;
pub mod qa_score;
