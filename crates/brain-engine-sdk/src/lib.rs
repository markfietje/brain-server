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

pub mod host;
pub mod policy;
pub mod pure;
