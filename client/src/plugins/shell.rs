//! **ui-shell** — the layout plugin: nav rail, topbar, theme/locale. Owns the
//! `ctx.layout` service and is the ONLY plugin that declares `root`; everyone
//! else composes through slots beneath it.

use super::{CTX_LAYOUT, Plugin, PluginCtx, PluginError};

pub struct Shell;

impl Plugin for Shell {
    fn name(&self) -> &'static str {
        "ui-shell"
    }
    fn ctx_key(&self) -> &'static str {
        CTX_LAYOUT
    }
    fn declares(&self) -> &'static [&'static str] {
        &["root"]
    }
    fn mount(&self, ctx: &mut PluginCtx) -> Result<(), PluginError> {
        use crate::slots::slot_names::Root;
        // The shell's own render seat — declared children hang off this entry.
        ctx.register::<Root>(
            crate::slots::SlotSpec::new("app", 0).with_store(serde_json::json!({"children": true})),
        )
    }
}
