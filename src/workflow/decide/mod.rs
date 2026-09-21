//! The System-One decide modules — the Phase 0 PURE port
//! (LAYA_RUST_PORT §2): dependency-free (`std` + `serde` + `serde_json`
//! only), total functions, no I/O, no threads, no env, no clock, no
//! model. Advisory posture: closed vocabularies stay closed, escalation
//! on any doubt, and nothing here changes runtime behavior — the modules
//! compile ungated and gain callers only when the lane opens (Phase 1+).
//!
//! Not in this round: `inference.rs`/`audit_ext.rs` (feature-gated) and
//! the `laya-local` Cargo feature — Phase 1, never prebuilt.

pub(crate) mod calibration;
pub(crate) mod lang;
pub(crate) mod presets;
pub(crate) mod router;
pub(crate) mod sequence;

pub(crate) use router::{RouteDecision, route};
pub(crate) use sequence::{QType, TypedQuestion};

#[cfg(test)]
mod tests {
    use super::*;

    /// The module surface re-exports the pure core (the doc's §4.4 API at
    /// pub(crate) visibility).
    #[test]
    fn decide_module_reexports_the_pure_core() {
        let q = TypedQuestion::noul("smoke");
        let _: Vec<String> = crate::workflow::decide::sequence::render_options(&q);
        let decision: Result<RouteDecision, String> = route(
            &std::collections::BTreeSet::new(),
            Some("english"),
            None,
            None,
            false,
            "english",
            false,
            &lang::analyse(&serde_json::json!("the battery works")),
        );
        assert_eq!(decision.unwrap().model, "english");
        assert_eq!(QType::Choice as i32, 0);
        assert_eq!(QType::Score as i32, 1);
        assert_eq!(QType::Noul as i32, 2);
    }
}
