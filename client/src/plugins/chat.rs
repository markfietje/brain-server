//! **ui-chat** — the conversation surface plugin: conversation view, composer,
//! input docks, and the keyed chat-node dispatch (with a generic-card fallback
//! owned here). Publishes `ctx.conversation`. The HITL `review-job` node view
//! is deliberately NOT registered here — ui-control-panel mounts its own
//! renderer into the keyed hole.

use super::{CTX_CONVERSATION, Plugin, PluginCtx, PluginError};

pub struct Chat;

/// The chat-node kinds this plugin renders itself (the generic + built-in set).
pub const BUILTIN_NODE_KINDS: &[&str] = &["assistant", "tool", "delivery", "workflow-run"];

impl Plugin for Chat {
    fn name(&self) -> &'static str {
        "ui-chat"
    }
    fn ctx_key(&self) -> &'static str {
        CTX_CONVERSATION
    }
    fn declares(&self) -> &'static [&'static str] {
        &[
            "conversation.view",
            "conversation.input.dock",
            "conversation.chat.node",
            "conversation.details.tool",
        ]
    }
    fn mount(&self, ctx: &mut PluginCtx) -> Result<(), PluginError> {
        use crate::slots::slot_names::{ChatNodeSlot, ConversationView, InputDock};
        ctx.register::<ConversationView>(crate::slots::SlotSpec::new("main", 0))?;
        // The offline-queue dock sits at order 20; third parties insert between
        // the control panel's approval dock (5) and this.
        ctx.register::<InputDock>(crate::slots::SlotSpec::new("queue", 20))?;
        for kind in BUILTIN_NODE_KINDS {
            let spec = crate::slots::SlotSpec::new(kind, 0);
            // v1.28.19 Witness: the workflow-run registration carries its
            // surface metadata (lineage timeline + AskHuman card); the other
            // built-ins stay bare until their renderers land (Cockpit).
            if *kind == "workflow-run" {
                ctx.register::<ChatNodeSlot>(spec.with_store(serde_json::json!({
                    "surface": "conversation",
                    "timeline": true,
                    "askhuman": true,
                })))?;
            } else {
                ctx.register::<ChatNodeSlot>(spec)?;
            }
        }
        Ok(())
    }
}
