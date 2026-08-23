//! **ui-control-panel** — the HITL approvals plugin: review queue, digest-bound
//! decisions, SLA clocks, batch ops. Publishes `ctx.approvals` and mounts a
//! `conversation.input.dock` entry at order 5 plus the keyed
//! `conversation.chat.node` renderer for `review-job` nodes — approval
//! workflow as a chat plugin, with the server staying authoritative
//! (the plugin boundary changes presentation, never enforcement).

use super::{CTX_APPROVALS, Plugin, PluginCtx, PluginError};

pub const APPROVAL_DOCK_KEY: &str = "approval";
pub const REVIEW_NODE_KIND: &str = "review-job";
/// Dock order 5: between todo/goal entries and the queue dock (20).
pub const APPROVAL_DOCK_ORDER: i32 = 5;

pub struct ControlPanel;

impl Plugin for ControlPanel {
    fn name(&self) -> &'static str {
        "ui-control-panel"
    }
    fn ctx_key(&self) -> &'static str {
        CTX_APPROVALS
    }
    fn declares(&self) -> &'static [&'static str] {
        &["settings.section"]
    }
    fn mount(&self, ctx: &mut PluginCtx) -> Result<(), PluginError> {
        use crate::slots::slot_names::{ChatNodeSlot, InputDock, SettingsSection};
        ctx.register::<SettingsSection>(
            crate::slots::SlotSpec::new("approvals", 0)
                .with_store(serde_json::json!({"title": "approvals"})),
        )?;
        // Role-gated visibility is fail-closed: unknown predicates never grant.
        ctx.register::<InputDock>(
            crate::slots::SlotSpec::new(APPROVAL_DOCK_KEY, APPROVAL_DOCK_ORDER)
                .when("role:approve"),
        )?;
        ctx.register::<ChatNodeSlot>(
            crate::slots::SlotSpec::new(REVIEW_NODE_KIND, 0)
                // v1.28.19 Witness: the keyed dispatch carries real metadata —
                // the conversation surface renders the digest-bound decision
                // card through this registration.
                .with_store(serde_json::json!({
                    "surface": "conversation",
                    "decide": true,
                    "digest_bound": true,
                })),
        )?;
        Ok(())
    }
}
