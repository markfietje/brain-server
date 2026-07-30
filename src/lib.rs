//! brain-server library target.
//!
//! Exists so connector binaries (`brain-connector-gh`, future connectors)
//! and tooling binaries (`brain-migrate-rehearse`) can share modules via
//! `use brain_server::...` instead of `#[path]`-including each file
//! individually. The server binary (`src/main.rs`) is unaffected — it has
//! its own `mod` declarations for in-process use.
//!
//! Only cross-cutting modules are exposed here. Server-specific modules
//! (`main.rs`, `handlers/`, `search/`, etc.) stay private to the server binary.

pub mod connector;

// Audit log + backup/restore (v0.9.7 "Guard"): shared cross-cutting concerns
// exposed to the `brain` CLI binary. `audit` is used by `backup` for
// lifecycle events and by the CLI for connector-registration events.
pub mod audit;
pub mod backup;

// Storage layout (v0.9.9 "Qualify" M1.1): the single source of truth for every
// on-disk path brain-server touches. Exposed so `brain-migrate-rehearse` and
// future tooling derive the same paths as the server. `domain_registry`
// (server-private) delegates its `is_valid_domain` to this module.
pub mod storage_layout;

// Capacity envelopes (v0.9.9 "Qualify" M3.1): the published limits the server
// enforces via HTTP 507 on writes and reports via `/health`. Lives in the lib
// so `bench` + `brain-migrate-rehearse` assert against the same envelope.
pub mod capacity;

// Schema migration (v0.9.9 "Qualify" M2.1): extracted from `main.rs` so the
// `brain-migrate-rehearse` binary can bring old-schema fixtures up to current.
// Server-internal callers (`run_migration` at startup, in tests) use it via
// `brain_server::migration::`.
pub mod migration;

// Retrieval-quality metrics (v1.4.0 "Calibrate" M5): pure P@k/R@k/MRR/NDCG/
// answer_in_context functions for the regression bench harness. Lives in the
// lib so the `bench` binary (feature-gated) consumes them without a #[path]
// include. The 100-query hand-judged corpus is an operator step; these
// functions are the reproducible engine any judgments file plugs into.
pub mod eval;
