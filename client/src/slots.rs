//! The slot system: every mountable UI region is a registry entry, never a
//! hardcoded component import. Cross-plugin cooperation is registration +
//! ordered lookup, so third parties extend the shell without editing it.
//!
//! Contract:
//! - `SlotKind` names a slot family (`const NAME`); `SlotSpec` is one entry in
//!   it ("who injects it, owns its shape" — entries carry an opaque store).
//! - `ordered()` is the render order; duplicate keys replace (idempotent
//!   re-registration on hot reload), unknown slot families are rejected.
//! - `changed()` is the revision counter backing the `slots/changed` event;
//!   the renderer subscribes instead of diffing trees.
//!
//! Port note: semantics ported from the harness slot-map pattern; original
//! Rust, compile-time extension only (no JS loader — honest ceiling).
//!
//! Truthful allow: the registry core is live (the approval dock composes
//! through it); the remaining families (`settings.section`, `conversation.*`,
//! `tool.call.toolview`) are the declared extension surface for plugins that
//! mount when a consumer registers them — same one-release-ahead posture.

#![allow(dead_code)]

use std::collections::HashMap;

/// A slot family. Declare per family; `NAME` doubles as the registry key.
pub trait SlotKind {
    const NAME: &'static str;
}

pub mod slot_names {
    use super::SlotKind;
    pub struct SettingsSection;
    impl SlotKind for SettingsSection {
        const NAME: &'static str = "settings.section";
    }
    pub struct ConversationView;
    impl SlotKind for ConversationView {
        const NAME: &'static str = "conversation.view";
    }
    pub struct ChatNodeSlot;
    impl SlotKind for ChatNodeSlot {
        const NAME: &'static str = "conversation.chat.node";
    }
    pub struct ToolCallView;
    impl SlotKind for ToolCallView {
        const NAME: &'static str = "tool.call.toolview";
    }
    pub struct InputDock;
    impl SlotKind for InputDock {
        const NAME: &'static str = "conversation.input.dock";
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Visibility {
    Always,
    /// Rendered only when the predicate string matches the shell capability
    /// set (e.g. `"role:dpo"`). Unknown predicates hide the entry — fail-closed.
    When(String),
}

#[derive(Debug, Clone)]
pub struct SlotSpec {
    /// Stable key within the family; re-registering a key replaces.
    pub key: String,
    pub order: i32,
    pub visibility: Visibility,
    /// Owner-typed payload (the "store" share). Opaque to the registry.
    pub store: serde_json::Value,
}

impl SlotSpec {
    pub fn new(key: &str, order: i32) -> Self {
        Self {
            key: key.to_string(),
            order,
            visibility: Visibility::Always,
            store: serde_json::Value::Null,
        }
    }
    pub fn when(mut self, pred: &str) -> Self {
        self.visibility = Visibility::When(pred.to_string());
        self
    }
    pub fn with_store(mut self, v: serde_json::Value) -> Self {
        self.store = v;
        self
    }
}

#[derive(Debug, Default)]
pub struct SlotRegistry {
    entries: HashMap<String, Vec<SlotSpec>>,
    revision: u64,
}

impl SlotRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register into the family named by `K`. Duplicate keys replace in place
    /// (declaration merging); order is stable otherwise. Bumps the revision.
    pub fn register<K: SlotKind>(&mut self, spec: SlotSpec) {
        let list = self.entries.entry(K::NAME.to_string()).or_default();
        if let Some(slot) = list.iter_mut().find(|e| e.key == spec.key) {
            *slot = spec;
        } else {
            list.push(spec);
        }
        self.revision += 1;
    }

    /// Fail-closed: an unregistered family has no entries and cannot be created
    /// by lookup (only `register<K>` mints families).
    pub fn ordered<K: SlotKind>(&self) -> Vec<&SlotSpec> {
        let mut v: Vec<&SlotSpec> = self
            .entries
            .get(K::NAME)
            .map(|l| l.iter().collect())
            .unwrap_or_default();
        v.sort_by_key(|e| e.order);
        v
    }

    /// Keyed family view (chat nodes by kind, tool views by name).
    pub fn keyed<K: SlotKind>(&self, key: &str) -> Option<&SlotSpec> {
        self.entries.get(K::NAME)?.iter().find(|e| e.key == key)
    }

    /// Remove an entry (unmount lifecycle).
    pub fn deregister<K: SlotKind>(&mut self, key: &str) -> bool {
        if let Some(list) = self.entries.get_mut(K::NAME) {
            let before = list.len();
            list.retain(|e| e.key != key);
            if list.len() != before {
                self.revision += 1;
                return true;
            }
        }
        false
    }

    /// Monotonic change counter — the `slots/changed` payload.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// The standard visibility evaluation: `Always` renders; `When(pred)` renders
/// only on an exact capability-match against the shell's granted set. Unknown
/// predicates hide (fail-closed).
pub fn visible(spec: &SlotSpec, capabilities: &[String]) -> bool {
    match &spec.visibility {
        Visibility::Always => true,
        Visibility::When(p) => capabilities.iter().any(|c| c == p),
    }
}

/// Ordered + visibility-filtered render set for a family.
pub fn render_set<'a, K: SlotKind>(reg: &'a SlotRegistry, caps: &[String]) -> Vec<&'a SlotSpec> {
    reg.ordered::<K>()
        .into_iter()
        .filter(|s| visible(s, caps))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slot_names::*;

    #[test]
    fn ordering_and_declaration_merging() {
        let mut r = SlotRegistry::new();
        r.register::<SettingsSection>(SlotSpec::new("b", 10));
        r.register::<SettingsSection>(SlotSpec::new("a", 2));
        let ord = r.ordered::<SettingsSection>();
        assert_eq!(ord[0].key, "a");
        // Same key replaces, no duplicate row appears.
        r.register::<SettingsSection>(SlotSpec::new("a", 1));
        assert_eq!(r.ordered::<SettingsSection>().len(), 2);
        assert_eq!(r.ordered::<SettingsSection>()[0].order, 1);
    }

    #[test]
    fn unknown_family_is_empty_fail_closed() {
        let r = SlotRegistry::new();
        assert!(r.ordered::<ToolCallView>().is_empty());
    }

    #[test]
    fn keyed_lookup() {
        let mut r = SlotRegistry::new();
        r.register::<ChatNodeSlot>(SlotSpec::new("review-job", 0));
        assert!(r.keyed::<ChatNodeSlot>("review-job").is_some());
        assert!(r.keyed::<ChatNodeSlot>("nope").is_none());
    }

    #[test]
    fn visibility_is_fail_closed() {
        let mut r = SlotRegistry::new();
        r.register::<InputDock>(SlotSpec::new("open", 0));
        r.register::<InputDock>(SlotSpec::new("dpo-only", 5).when("role:dpo"));
        let none: Vec<String> = vec![];
        assert_eq!(render_set::<InputDock>(&r, &none).len(), 1);
        let dpo = vec!["role:dpo".to_string()];
        assert_eq!(render_set::<InputDock>(&r, &dpo).len(), 2);
        // An unknown predicate never grants.
        let weird = vec!["role:dpo ".to_string()];
        assert_eq!(render_set::<InputDock>(&r, &weird).len(), 1);
    }

    #[test]
    fn deregister_and_revision() {
        let mut r = SlotRegistry::new();
        let rev0 = r.revision();
        r.register::<ConversationView>(SlotSpec::new("main", 0));
        assert!(r.revision() > rev0);
        assert!(r.deregister::<ConversationView>("main"));
        assert!(
            !r.deregister::<ConversationView>("main"),
            "second remove is a no-op"
        );
    }

    #[test]
    fn twenty_slot_mount_is_bounded() {
        let mut r = SlotRegistry::new();
        let t0 = std::time::Instant::now();
        for i in 0..20 {
            r.register::<SettingsSection>(SlotSpec::new(&format!("s{i}"), i));
        }
        let set = render_set::<SettingsSection>(&r, &[]);
        assert_eq!(set.len(), 20);
        assert!(
            t0.elapsed().as_millis() < 50,
            "20-slot register+render must stay well under 50ms"
        );
    }
}
