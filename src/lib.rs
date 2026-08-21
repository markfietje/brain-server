#![allow(deprecated)]
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

// Audit log + backup/restore: shared cross-cutting concerns
// exposed to the `brain` CLI binary. `audit` is used by `backup` for
// lifecycle events and by the CLI for connector-registration events.
pub mod audit;
pub mod backup;

// Storage layout: the single source of truth for every
// on-disk path brain-server touches. Exposed so `brain-migrate-rehearse` and
// future tooling derive the same paths as the server. `domain_registry`
// (server-private) delegates its `is_valid_domain` to this module.
pub mod storage_layout;

// Capacity envelopes: the published limits the server
// enforces via HTTP 507 on writes and reports via `/health`. Lives in the lib
// so `bench` + `brain-migrate-rehearse` assert against the same envelope.
pub mod capacity;

// Schema migration: extracted from `main.rs` so the
// `brain-migrate-rehearse` binary can bring old-schema fixtures up to current.
// Server-internal callers (`run_migration` at startup, in tests) use it via
// `brain_server::migration::`.
pub mod migration;

// Retrieval-quality metrics: pure P@k/R@k/MRR/NDCG/
// answer_in_context functions for the regression bench harness. Lives in the
// lib so the `bench` binary (feature-gated) consumes them without a #[path]
// include. The 100-query hand-judged corpus is an operator step; these
// functions are the reproducible engine any judgments file plugs into.
pub mod eval;

// UMP 1.0 integrity + identity: base32/did:key/JCS/
// BLAKE3/Ed25519/capability-token primitives. Pure functions, shared by the
// server (`/ump/*`, sign-on-write) and the `brain` CLI (`brain ump keygen`,
// `brain ump export`) — same cross-binary pattern as `eval`/`capacity`.
pub mod ump_integrity;

// sqlite-vec registration (hardening pass): the single audited, correctly-
// typed FFI call that both the server binary and `brain-migrate-rehearse`
// use to register sqlite-vec process-wide. Lives in the lib so the two
// binaries never duplicate an `unsafe` block (the pre-hardening state).
pub mod register_sqlite_vec;

// The one invisible-Unicode strip boundary: shared by the
// MCP binary + `brain` CLI so every agent-facing surface closes the same
// bidi/zero-width smuggling class as the server screen. The server binary's
// `screen.rs` re-exports these so `crate::screen::*` paths stay unchanged.
pub mod strip_invisible;

// Edge supersession: the pure write-path core that makes
// `relationships` true to the bi-temporal contract `trace` documents.
// Lives in the lib so a bare-`Connection` unit test can
// drive it (the `page_decayed` idiom) and so the server handler only wires.
pub mod graph_supersede;

// The shared untrusted-fence primitives: the
// sentinel constants + the markdown-ref strip the MCP binary and CLI wrap
// agent-bound text with. `src/gate.rs` re-exports `strip_markdown_refs` so
// the server surface keeps its existing call path (single definition).
pub mod fence;

// The embedding abstraction: the trait + the static
// (default) backend + the feature-gated neural (bge-m3) backend. Lives in the
// lib so `bench` consumes it without a #[path] include, same pattern as
// `eval`/`capacity`. `AppState` rewiring is the gated follow-up.
pub mod embed;

// the preset bundle type + persistence + the 12
// ship-with presets. Lives in the lib because `migration` (lib) seeds the
// presets at first boot and the `brain` CLI renders them for `brain setup`.
pub mod profile;

// the role bundle type (scopes + owner_filter + `can` +
// panel visibility + MCP tools), persistence, the 10 ship-with roles, and the
// record-level retrieval gate. Lives in the lib because `migration` (lib)
// seeds the presets at first boot and `handlers`/`mcp` (server binaries)
// enforce the gate.
pub mod role;
