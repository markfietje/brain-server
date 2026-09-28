//! the RBAC evaluation middleware: a closed, deterministic oracle and the
//! seam that runs it.
//!
//! **The shape of the round.** Two layers, one decision. `policy` is the oracle:
//! pure, total, closed-vocabulary, zero I/O, zero transport types. `gates` is
//! the declaration the runtime and the coverage pin BOTH read, so the two
//! cannot drift. The middleware itself is a protocol adapter and therefore
//! lives in `server::router::auth`, beside the two middlewares it runs after —
//! this module stays transport-free so a policy oracle is never wired to HTTP.
//!
//! **What the middleware enforces, stated exactly, because the honest answer
//! is narrower than the round's ambition:**
//!
//!   * **route coverage** — a matched, non-public route with no row in the
//!     shared table is `Deny(RouteUngated)`. This is the round's reason for
//!     existing: the coverage property becomes a property of the running
//!     server instead of a claim about a test, and a route nobody remembered
//!     to gate is now a route that cannot be served.
//!   * **the agent class on Admin rows** — refused by class, before any role
//!     is consulted.
//!   * **the deny-only capability class** — a capability outside
//!     `CAN_ACTIONS` is a permanent refusal, declared rather than implied.
//!
//! **What it does NOT enforce, and this is a decision, not an omission:** the
//! per-route CAPABILITY and the scope ACTION stay with the handler's own
//! `authorize` / `authorize_role` (the defence-in-depth rule's inner gate). The capability cannot move
//! here: the KCS publish gate is conditional on a request body field inside a
//! route that carries two other gates, and a middleware keyed on
//! `(MatchedPath, Method)` cannot see a body. The action axis already agrees
//! with the handlers by construction — the authz matrix pins each row to the
//! `authorize()` literal its handler actually calls — so re-deriving it here
//! would be a second opinion on a decision already made once. The middleware
//! is an outer filter over a property that could not otherwise be enforced, and
//! the handlers remain the inner gate.
//!
//! **Fail-closed, with one measured exception .** Every unknown the
//! middleware owns is a denial with a named reason. The exception is the absent
//! `Principal`, which is a DEFERRAL: `src/server/router/auth.rs` admits the
//! opaque operator token without inserting one, and
//! `tests/authz_matrix.rs::single_token_legacy_posture_unchanged` pins that this
//! token keeps reaching Admin routes. Authentication has already ruled; see
//! `policy`'s module doc.

#![forbid(unsafe_code)]

pub mod gates;
pub mod policy;

pub use gates::{
    DENY_ONLY_CAPABILITIES, PRESENTATION_GATED, gate_for, is_deny_only, is_presentation_gated,
};
pub use policy::{DeferReason, DenyReason, Gate, Verdict, decide_gate_verdict};
