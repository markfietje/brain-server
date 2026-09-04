//! The server seam — the Spire Line's thin-bin target.
//!
//! `bootstrap`: env/config resolution, fail-closed checks, pool + state
//! construction, watchdog spawns — protocol-free. `router`: the route
//! families + the composed `app(state)`.
//!
//! Law: `src/server/**` contains ONLY these modules (no new submodules
//! beyond the six router families); bootstrap takes no axum types.

pub(crate) mod bootstrap;
pub(crate) mod router;
