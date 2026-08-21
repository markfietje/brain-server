// Truthful allow (workflow-substrate precedent): scaffold lands one release ahead of its UI consumer.
#![allow(dead_code)]

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ChatNode {
    pub key: String,
    pub kind: String,
    pub seq: u64,
    pub data: serde_json::Value,
}

pub struct Assembler {
    nodes: HashMap<String, ChatNode>,
    order: Vec<String>,
}

impl Assembler {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            order: Vec::new(),
        }
    }
    pub fn ingest(&mut self, seq: u64, kind: &str, id: &str, data: serde_json::Value) {
        let key = format!("{kind}:{id}");
        if !self.nodes.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.nodes.insert(
            key.clone(),
            ChatNode {
                key,
                kind: kind.to_string(),
                seq,
                data,
            },
        );
        self.order.sort_by_key(|k| self.nodes[k].seq);
    }
    pub fn snapshot(&self) -> Vec<&ChatNode> {
        self.order
            .iter()
            .filter_map(|k| self.nodes.get(k))
            .collect()
    }
}

impl Default for Assembler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn assembler_ordering() {
        let mut a = Assembler::new();
        a.ingest(2, "tool-call", "1", serde_json::json!({}));
        a.ingest(1, "assistant", "1", serde_json::json!({}));
        assert_eq!(a.snapshot()[0].kind, "assistant");
    }
    #[test]
    fn assembler_update() {
        let mut a = Assembler::new();
        a.ingest(
            1,
            "review-job",
            "42",
            serde_json::json!({"status":"running"}),
        );
        a.ingest(
            1,
            "review-job",
            "42",
            serde_json::json!({"status":"completed"}),
        );
        assert_eq!(a.snapshot().len(), 1);
        assert_eq!(a.snapshot()[0].data["status"], "completed");
    }
}
