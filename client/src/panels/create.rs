//! Create workspace (v1.17.7 M4, Write group): a single `/create` page that
//! stacks the three write tools — Ingest, Procedures, Consolidate. One page,
//! one nav target, no tab chrome; each panel is self-contained.

use crate::panels::{use_document_title, PageTitle};
use dioxus::prelude::*;

pub fn panel() -> Element {
    use_document_title(|| "Create — brain".into());
    let sub = crate::i18n::t("create_sub");
    rsx! {
        PageTitle { {crate::i18n::t("create_title")} }
        p { class: "text-sm text-muted-foreground mb-4", "{sub}" }
        { crate::panels::ingest::panel() }
        { crate::panels::procedures::panel() }
        { crate::panels::consolidate::panel() }
    }
}
