//! The conversation event registry: definitions with a unique `KIND` register
//! under lifecycle (mount → register, unmount → deregister). The fallback is
//! the sole generic card — exactly one, never per-definition.

use super::{NodeDefinition, Publication};
use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct EventRegistry {
    kinds: HashSet<&'static str>,
}

impl EventRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register on mount. Duplicate kind registration is refused (unique-kind
    /// invariant) — returns false instead of silently overwriting.
    pub fn register<D: NodeDefinition>(&mut self) -> bool {
        self.kinds.insert(D::KIND)
    }

    /// Deregister on unmount.
    pub fn deregister<D: NodeDefinition>(&mut self) -> bool {
        self.kinds.remove(D::KIND)
    }

    pub fn is_registered(&self, kind: &str) -> bool {
        self.kinds.contains(kind)
    }

    /// Publication policy lookup; unregistered kinds have none (`None`).
    pub fn publication_of(&self, kind: &str) -> Option<Publication> {
        // The built-in families carry their policy as consts; this registry is
        // the single place callers resolve it from.
        use super::*;
        let policy = match kind {
            k if k == AssistantNode::KIND => AssistantNode::PUBLICATION,
            k if k == ToolNode::KIND => ToolNode::PUBLICATION,
            k if k == ReviewJobNode::KIND => ReviewJobNode::PUBLICATION,
            k if k == DeliveryNode::KIND => DeliveryNode::PUBLICATION,
            k if k == WorkflowRunNode::KIND => WorkflowRunNode::PUBLICATION,
            _ => return None,
        };
        self.is_registered(kind).then_some(policy)
    }

    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}

/// The built-in registration set (the default chat surface).
pub fn builtin_registry() -> EventRegistry {
    let mut r = EventRegistry::new();
    assert!(r.register::<super::AssistantNode>());
    assert!(r.register::<super::ToolNode>());
    assert!(r.register::<super::ReviewJobNode>());
    assert!(r.register::<super::DeliveryNode>());
    assert!(r.register::<super::WorkflowRunNode>());
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{AssistantNode, ReviewJobNode};

    #[test]
    fn unique_kind_invariant() {
        let mut r = EventRegistry::new();
        assert!(r.register::<AssistantNode>());
        assert!(!r.register::<AssistantNode>(), "duplicate kind refused");
    }

    #[test]
    fn lifecycle_register_deregister() {
        let mut r = EventRegistry::new();
        assert!(r.register::<ReviewJobNode>());
        assert!(r.is_registered("review-job"));
        assert!(r.deregister::<ReviewJobNode>());
        assert!(!r.is_registered("review-job"));
        assert!(r.publication_of("review-job").is_none());
    }

    #[test]
    fn publication_policies_resolve() {
        let r = builtin_registry();
        assert_eq!(r.len(), 5);
        assert_eq!(
            r.publication_of("assistant"),
            Some(Publication::AnimationFrame)
        );
        assert_eq!(r.publication_of("review-job"), Some(Publication::Immediate));
        assert_eq!(r.publication_of("alien"), None);
    }
}
