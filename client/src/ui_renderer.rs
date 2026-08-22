//! The UI renderer contract: the shell renders only `root`; everything else
//! mounts through slot registration. A family's render set is derived from the
//! registry (order + fail-closed visibility), never from hardcoded imports.
//!
//! Chat dispatch goes through `conversation.chat.node` keyed by node kind with
//! a generic-card fallback, so a kind with no registered view still renders
//! (never silently drops).
//!
//! Truthful allow: dock composition is live (the approval dock); keyed chat
//! dispatch mounts with the streaming conversation surface.

#![allow(dead_code)]

use crate::slots::{
    SlotKind, SlotRegistry, SlotSpec, render_set,
    slot_names::{ChatNodeSlot, InputDock},
};

/// One materialized render target: a slot entry resolved against capabilities.
#[derive(Debug, Clone)]
pub struct RenderEntry {
    pub family: &'static str,
    pub key: String,
    pub order: i32,
    pub store: serde_json::Value,
}

impl<'a> From<(&'static str, &'a SlotSpec)> for RenderEntry {
    fn from((family, s): (&'static str, &'a SlotSpec)) -> Self {
        Self {
            family,
            key: s.key.clone(),
            order: s.order,
            store: s.store.clone(),
        }
    }
}

fn family_name<K: SlotKind>() -> &'static str {
    K::NAME
}

/// Resolve a family to ordered render entries (visibility applied).
pub fn resolve<K: SlotKind>(reg: &SlotRegistry, caps: &[String]) -> Vec<RenderEntry> {
    let family = family_name::<K>();
    render_set::<K>(reg, caps)
        .into_iter()
        .map(|s| (family, s).into())
        .collect()
}

/// Chat-node dispatch: exact keyed view first, else `None` (the caller renders
/// its generic card — the renderer never invents views).
pub fn chat_node_view<'a>(reg: &'a SlotRegistry, node_kind: &str) -> Option<&'a SlotSpec> {
    reg.keyed::<ChatNodeSlot>(node_kind)
}

/// Input-dock composition: the dock order is data (`TodoDock`=0, `ApprovalDock`
/// =5, goal=10, queue=20 by convention), each hides when absent, third parties
/// insert between orders.
pub fn input_docks(reg: &SlotRegistry, caps: &[String]) -> Vec<RenderEntry> {
    resolve::<InputDock>(reg, caps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slots::slot_names::ChatNodeSlot;

    fn reg() -> SlotRegistry {
        let mut r = SlotRegistry::new();
        r.register::<InputDock>(
            SlotSpec::new("queue", 20).with_store(serde_json::json!({"label":"Queue"})),
        );
        r.register::<InputDock>(SlotSpec::new("approval", 5));
        r
    }

    #[test]
    fn docks_render_in_order_with_store() {
        let docks = input_docks(&reg(), &[]);
        assert_eq!(docks[0].key, "approval");
        assert_eq!(docks[0].family, "conversation.input.dock");
        assert!(docks[1].store.is_object(), "owner share rides through");
    }

    #[test]
    fn chat_dispatch_is_exact_or_none() {
        let mut r = reg();
        r.register::<ChatNodeSlot>(SlotSpec::new("review-job", 0));
        assert!(chat_node_view(&r, "review-job").is_some());
        // Unknown kinds get no view — the caller falls back, nothing panics.
        assert!(chat_node_view(&r, "alien-kind").is_none());
    }

    #[test]
    fn visibility_filters_the_render_set() {
        let mut r = reg();
        r.register::<InputDock>(SlotSpec::new("secret", 1).when("role:dpo"));
        assert_eq!(input_docks(&r, &[]).len(), 2);
        assert_eq!(
            input_docks(&r, &["role:dpo".to_string()]).len(),
            3,
            "dpo sees the gated dock"
        );
    }
}
