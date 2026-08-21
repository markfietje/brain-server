// Truthful allow (workflow-substrate precedent): scaffold lands one release ahead of its UI consumer.
#![allow(dead_code)]

use std::collections::HashMap;

pub struct SlotSpec {
    pub name: String,
    pub key: Option<String>,
    pub order: i32,
}

pub struct SlotRegistry {
    entries: HashMap<String, Vec<SlotSpec>>,
}

impl SlotRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
    pub fn register(&mut self, spec: SlotSpec) {
        self.entries
            .entry(spec.name.clone())
            .or_default()
            .push(spec);
    }
    pub fn entries_for(&self, name: &str) -> Option<&Vec<SlotSpec>> {
        self.entries.get(name)
    }
    pub fn ordered(&self, name: &str) -> Vec<&SlotSpec> {
        let mut v: Vec<&SlotSpec> = self
            .entries
            .get(name)
            .map(|x| x.iter().collect())
            .unwrap_or_default();
        v.sort_by_key(|e| e.order);
        v
    }
}

impl Default for SlotRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_ordering() {
        let mut r = SlotRegistry::new();
        r.register(SlotSpec {
            name: "settings.section".into(),
            key: Some("a".into()),
            order: 10,
        });
        r.register(SlotSpec {
            name: "settings.section".into(),
            key: Some("b".into()),
            order: 2,
        });
        assert_eq!(r.ordered("settings.section")[0].order, 2);
    }
    #[test]
    fn reversible_unmount() {
        let mut r = SlotRegistry::new();
        r.register(SlotSpec {
            name: "x".into(),
            key: None,
            order: 0,
        });
        assert!(r.entries_for("x").is_some());
        r.entries.remove("x");
        assert!(r.entries_for("x").is_none());
    }
}
