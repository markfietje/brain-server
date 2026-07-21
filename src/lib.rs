//! brain-server library target.
//!
//! Exists so connector binaries (`brain-connector-gh`, future connectors)
//! can share the connector modules via `use brain_server::connector::...`
//! instead of `#[path]`-including each file individually. The server binary
//! (`src/main.rs`) is unaffected — it has its own `mod connector;` declaration
//! for in-process use.
//!
//! Only the connector module tree is exposed here. Server-specific modules
//! (`main.rs`, `handlers/`, `search/`, etc.) stay private to the server binary.

pub mod connector;

// Audit log + backup/restore (v0.9.7 "Guard"): shared cross-cutting concerns
// exposed to the `brain` CLI binary. `audit` is used by `backup` for
// lifecycle events and by the CLI for connector-registration events.
pub mod audit;
pub mod backup;
