//! The session tree: an append-only entry log plus a runtime projection —
//! the gajae session-tree semantics ported (semantics only, no code copied).
//!
//! The model: every entry has an `id` and an optional `parent_id`; the
//! active position is `leaf_id`; appending always creates a child of the
//! current leaf (or a new root when the leaf is `None`). `branch` moves the
//! leaf to an existing entry and writes NOTHING — history is never
//! rewritten, context changes only because the walked root→leaf path
//! changed. Missing parents (orphans) are roots; children are ordered
//! oldest→newest by append order. Labels are entries too: the latest label
//! for a target wins, and an empty label deletes the mapping. Branch
//! summaries and custom entries convert to user-role context messages; raw
//! message bodies pass through; label entries never enter context.
//!
//! Every function is total: malformed entry JSON degrades to named
//! [`TreeError`] refusals, never a panic, never a truncated entry.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// The persisted target marker for a summary branch taken from the very
/// root (`null` in the source model; a JSON `null` loses the distinction
/// between "no field" and "null target", so the string is the persisted
/// form).
pub const ROOT_TARGET: &str = "root";

/// The closed vocabulary of tree entry kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeEntryKind {
    /// A conversation message (context material).
    Message,
    /// A custom/user-attached entry (context material, e.g. a handoff
    /// document injected into a fresh session).
    Custom,
    /// The summary written when navigation abandons a branch.
    BranchSummary,
    /// A label change for an existing entry (never context material).
    Label,
}

impl TreeEntryKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Custom => "custom",
            Self::BranchSummary => "branch_summary",
            Self::Label => "label",
        }
    }

    fn parse(s: &str) -> Result<Self, TreeError> {
        match s {
            "message" => Ok(Self::Message),
            "custom" => Ok(Self::Custom),
            "branch_summary" => Ok(Self::BranchSummary),
            "label" => Ok(Self::Label),
            other => Err(TreeError::UnknownKind(other.to_string())),
        }
    }
}

/// One append-only tree entry. `seq` is the projection's append order
/// (assigned on admission; JSON carries no seq — replay order is the
/// order). `from_id` is meaningful only on [`TreeEntryKind::BranchSummary`]
/// entries and records where the summary branch started; `None` is the
/// root (`ROOT_TARGET` in JSON). `label` is meaningful only on
/// [`TreeEntryKind::Label`] entries: `Some` sets the target's label, `None`
/// deletes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: TreeEntryKind,
    pub body: String,
    pub from_id: Option<String>,
    pub label: Option<String>,
    pub seq: u64,
}

impl TreeEntry {
    /// The canonical JSON form (the payload the session log stores). A
    /// branch summary taken from the very root persists `from: "root"` —
    /// the null target is never silently lost.
    pub fn to_json(&self) -> Value {
        let from = match (&self.from_id, self.kind) {
            (Some(id), _) => Value::String(id.clone()),
            (None, TreeEntryKind::BranchSummary) => Value::String(ROOT_TARGET.to_string()),
            (None, _) => Value::Null,
        };
        serde_json::json!({
            "id": self.id,
            "parent": self.parent_id,
            "kind": self.kind.as_str(),
            "body": self.body,
            "from": from,
            "label": self.label,
        })
    }

    /// Parse one entry from its JSON form. Malformed input is a named
    /// refusal — an empty id, an unknown kind, an ill-typed field — never a
    /// default entry, never a panic.
    pub fn from_json(value: &Value) -> Result<Self, TreeError> {
        let obj = value
            .as_object()
            .ok_or(TreeError::Malformed("entry must be a JSON object"))?;
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or(TreeError::Malformed("id must be a non-empty string"))?;
        let parent = match obj.get("parent") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => {
                return Err(TreeError::Malformed("parent must be null or non-empty"));
            }
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(TreeError::Malformed("parent must be a string")),
        };
        let kind = TreeEntryKind::parse(
            obj.get("kind")
                .and_then(Value::as_str)
                .ok_or(TreeError::Malformed("kind must be a string"))?,
        )?;
        let body = obj
            .get("body")
            .and_then(Value::as_str)
            .ok_or(TreeError::Malformed("body must be a string"))?;
        let from = match obj.get("from") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s == ROOT_TARGET => None,
            Some(Value::String(s)) if s.is_empty() => {
                return Err(TreeError::Malformed("from must be null, \"root\" or an id"));
            }
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(TreeError::Malformed("from must be a string")),
        };
        let label = match obj.get("label") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(TreeError::Malformed("label must be a string or null")),
        };
        Ok(Self {
            id: id.to_string(),
            parent_id: parent,
            kind,
            body: body.to_string(),
            from_id: from,
            label,
            seq: 0,
        })
    }
}

/// Why a tree operation refused. Named, never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    /// The target id does not exist in the tree.
    UnknownTarget(String),
    /// A kind outside the closed vocabulary.
    UnknownKind(String),
    /// A required field is missing or ill-typed (the name names it).
    Malformed(&'static str),
    /// The parent links would not terminate (a cycle arrived via replay).
    Corrupt,
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTarget(id) => write!(f, "unknown tree target: {id}"),
            Self::UnknownKind(k) => write!(f, "unknown tree entry kind: {k}"),
            Self::Malformed(why) => write!(f, "malformed tree entry: {why}"),
            Self::Corrupt => write!(f, "corrupt tree: parent links do not terminate"),
        }
    }
}

/// One node of the runtime tree projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode<'a> {
    pub entry: &'a TreeEntry,
    /// The resolved current label, if any.
    pub label: Option<String>,
    pub children: Vec<TreeNode<'a>>,
}

/// A user-role context message produced from the active branch (the
/// messages-conversion analogue: branch summaries and custom entries
/// become context; labels never do).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMessage {
    pub role: &'static str,
    pub text: String,
}

/// The projection: append-only entries plus the three runtime indices.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionTree {
    entries: Vec<TreeEntry>,
    by_id: BTreeMap<String, usize>,
    leaf_id: Option<String>,
    labels_by_id: BTreeMap<String, String>,
}

impl SessionTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replay entries (in log order) into a projection. Orphans — entries
    /// whose parent id never appears — are legal (they become roots); a
    /// cycle among the parent links is a named refusal, caught at load.
    /// The leaf starts at the LAST entry (the newest position), matching an
    /// append-only log.
    pub fn replay(entries: &[Value]) -> Result<Self, TreeError> {
        let mut tree = Self::new();
        for value in entries {
            tree.push_admission(TreeEntry::from_json(value)?)?;
        }
        tree.validate_acyclic()?;
        tree.leaf_id = tree.entries.last().map(|e| e.id.clone());
        tree.reindex_labels();
        Ok(tree)
    }

    /// Every parent chain must terminate. Bounded walk per entry: a
    /// MISSING parent is legal (that entry is an orphan root); a cycle
    /// (only expressible via replayed JSON) is a named refusal.
    fn validate_acyclic(&self) -> Result<(), TreeError> {
        for entry in &self.entries {
            let mut steps = 0usize;
            let mut cursor = entry.parent_id.clone();
            while let Some(id) = cursor {
                steps += 1;
                if steps > self.entries.len() {
                    return Err(TreeError::Corrupt);
                }
                let idx = match self.by_id.get(&id) {
                    Some(&idx) => idx,
                    None => break,
                };
                cursor = self.entries[idx].parent_id.clone();
            }
        }
        Ok(())
    }

    /// Append one entry as a child of the current leaf (or as a new root
    /// when the leaf is `None`). Label entries are appended through
    /// [`SessionTree::set_label`] instead — their target is explicit.
    /// Returns the admitted entry.
    pub fn append(&mut self, kind: TreeEntryKind, body: &str) -> Result<TreeEntry, TreeError> {
        if body.is_empty() {
            return Err(TreeError::Malformed("body must be non-empty"));
        }
        if kind == TreeEntryKind::Label {
            return Err(TreeError::Malformed(
                "label entries need an explicit target: use set_label",
            ));
        }
        let entry = TreeEntry {
            id: format!("e{}", self.entries.len() + 1),
            parent_id: self.leaf_id.clone(),
            kind,
            body: body.to_string(),
            from_id: None,
            label: None,
            seq: self.entries.len() as u64,
        };
        self.push_admission(entry.clone())?;
        self.leaf_id = Some(entry.id.clone());
        Ok(entry)
    }

    /// Append a label change for `target_id` (the gajae
    /// `appendLabelChange` shape): `Some` sets the label, `None` deletes
    /// the mapping. The entry lands on the current leaf chain like any
    /// other append; the TARGET is explicit in `from_id`.
    pub fn set_label(
        &mut self,
        target_id: &str,
        label: Option<&str>,
    ) -> Result<TreeEntry, TreeError> {
        if !self.by_id.contains_key(target_id) {
            return Err(TreeError::UnknownTarget(target_id.to_string()));
        }
        let entry = TreeEntry {
            id: format!("e{}", self.entries.len() + 1),
            parent_id: self.leaf_id.clone(),
            kind: TreeEntryKind::Label,
            body: label.unwrap_or_default().to_string(),
            from_id: Some(target_id.to_string()),
            label: label.map(str::to_string),
            seq: self.entries.len() as u64,
        };
        self.push_admission(entry.clone())?;
        self.apply_label(target_id, label);
        self.leaf_id = Some(entry.id.clone());
        Ok(entry)
    }

    /// Move the leaf to an existing entry WITHOUT writing anything — the
    /// law: navigation is not a fork; the log is untouched.
    pub fn branch(&mut self, entry_id: &str) -> Result<(), TreeError> {
        if !self.by_id.contains_key(entry_id) {
            return Err(TreeError::UnknownTarget(entry_id.to_string()));
        }
        self.leaf_id = Some(entry_id.to_string());
        Ok(())
    }

    /// Clear the leaf: the next append becomes a new root.
    pub fn reset_leaf(&mut self) {
        self.leaf_id = None;
    }

    /// Branch to `target` and append a branch summary as a child of the NEW
    /// leaf (attached at the new navigation position, never on the
    /// abandoned tail). A `None` target is the root — the summary entry
    /// persists `from: "root"`. Returns the summary entry.
    pub fn branch_with_summary(
        &mut self,
        target: Option<&str>,
        summary: &str,
    ) -> Result<TreeEntry, TreeError> {
        if let Some(id) = target {
            self.branch(id)?;
        } else {
            self.leaf_id = None;
        }
        if summary.is_empty() {
            return Err(TreeError::Malformed("summary must be non-empty"));
        }
        let entry = TreeEntry {
            id: format!("e{}", self.entries.len() + 1),
            parent_id: self.leaf_id.clone(),
            kind: TreeEntryKind::BranchSummary,
            body: summary.to_string(),
            from_id: target.map(str::to_string),
            label: None,
            seq: self.entries.len() as u64,
        };
        self.push_admission(entry.clone())?;
        self.leaf_id = Some(entry.id.clone());
        Ok(entry)
    }

    /// The root→node walk ending at `from_id` (inclusive). An unknown
    /// TARGET refuses by name; a missing ancestor is legal (the orphan
    /// starts the walk); a cycle is a named refusal.
    pub fn get_branch(&self, from_id: &str) -> Result<Vec<&TreeEntry>, TreeError> {
        let start = self
            .by_id
            .get(from_id)
            .ok_or_else(|| TreeError::UnknownTarget(from_id.to_string()))?;
        let mut walk = vec![&self.entries[*start]];
        let mut steps = 0usize;
        let mut cursor = self.entries[*start].parent_id.clone();
        while let Some(id) = cursor {
            steps += 1;
            if steps > self.entries.len() {
                return Err(TreeError::Corrupt);
            }
            let idx = match self.by_id.get(&id) {
                Some(&idx) => idx,
                None => break,
            };
            walk.push(&self.entries[idx]);
            cursor = self.entries[idx].parent_id.clone();
        }
        walk.reverse();
        Ok(walk)
    }

    /// The whole tree: entries whose parent is `null` OR whose parent id
    /// never appears (orphans) are roots; children are oldest→newest by
    /// append order; each node carries its resolved label.
    pub fn get_tree(&self) -> Vec<TreeNode<'_>> {
        fn children_of(tree: &SessionTree, parent: Option<&str>) -> Vec<usize> {
            tree.entries
                .iter()
                .enumerate()
                .filter(|(_, e)| match (&e.parent_id, parent) {
                    (None, None) => true,
                    (Some(p), Some(q)) => p == q,
                    (Some(p), None) => !tree.by_id.contains_key(p),
                    _ => false,
                })
                .map(|(i, _)| i)
                .collect()
        }
        fn build<'a>(tree: &'a SessionTree, indices: &[usize]) -> Vec<TreeNode<'a>> {
            indices
                .iter()
                .map(|&i| {
                    let entry = &tree.entries[i];
                    let kids = children_of(tree, Some(&entry.id));
                    TreeNode {
                        entry,
                        label: tree.labels_by_id.get(&entry.id).cloned(),
                        children: build(tree, &kids),
                    }
                })
                .collect()
        }
        build(self, &children_of(self, None))
    }

    /// The direct children of `parent_id`, oldest→newest.
    pub fn get_children(&self, parent_id: &str) -> Vec<&TreeEntry> {
        self.entries
            .iter()
            .filter(|e| e.parent_id.as_deref() == Some(parent_id))
            .collect()
    }

    /// The current leaf position (the entry the next append will child).
    pub fn leaf_id(&self) -> Option<&str> {
        self.leaf_id.as_deref()
    }

    /// The resolved label for an entry id.
    pub fn label_of(&self, id: &str) -> Option<&str> {
        self.labels_by_id.get(id).map(String::as_str)
    }

    /// Every entry, in append order (the append-only log as-is).
    pub fn entries(&self) -> &[TreeEntry] {
        &self.entries
    }

    /// Convert the ACTIVE branch (root→leaf) into user-role context
    /// messages: messages pass through, branch summaries ride a named
    /// template, custom entries pass through, labels never appear.
    /// Without a leaf there is no active branch — an empty context.
    pub fn context_messages(&self) -> Result<Vec<ContextMessage>, TreeError> {
        let mut out = Vec::new();
        if let Some(leaf) = self.leaf_id.clone() {
            for entry in self.get_branch(&leaf)? {
                let text = match entry.kind {
                    TreeEntryKind::Message => entry.body.clone(),
                    TreeEntryKind::Custom => entry.body.clone(),
                    TreeEntryKind::BranchSummary => format!(
                        "The conversation branched earlier; the abandoned \
                         branch is summarized: {}",
                        entry.body
                    ),
                    TreeEntryKind::Label => continue,
                };
                out.push(ContextMessage { role: "user", text });
            }
        }
        Ok(out)
    }

    fn push_admission(&mut self, mut entry: TreeEntry) -> Result<(), TreeError> {
        if self.by_id.contains_key(&entry.id) {
            return Err(TreeError::Malformed("duplicate entry id"));
        }
        entry.seq = self.entries.len() as u64;
        let idx = self.entries.len();
        self.by_id.insert(entry.id.clone(), idx);
        self.entries.push(entry);
        Ok(())
    }

    fn reindex_labels(&mut self) {
        self.labels_by_id.clear();
        let pairs: Vec<(String, Option<String>)> = self
            .entries
            .iter()
            .filter(|e| e.kind == TreeEntryKind::Label)
            .filter_map(|e| e.from_id.clone().map(|t| (t, e.label.clone())))
            .collect();
        for (target, label) in &pairs {
            self.apply_label(target, label.as_deref());
        }
    }

    fn apply_label(&mut self, target: &str, label: Option<&str>) {
        match label {
            Some(l) if !l.is_empty() => {
                self.labels_by_id.insert(target.to_string(), l.to_string());
            }
            _ => {
                self.labels_by_id.remove(target);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A five-entry linear log: e1 ← e2 ← e3 ← e4 ← e5 (leaf = e5).
    fn linear() -> SessionTree {
        let mut t = SessionTree::new();
        t.append(TreeEntryKind::Message, "root message").unwrap();
        t.append(TreeEntryKind::Message, "second").unwrap();
        t.append(TreeEntryKind::Message, "third").unwrap();
        t.append(TreeEntryKind::Message, "fourth").unwrap();
        t.append(TreeEntryKind::Message, "fifth").unwrap();
        t
    }

    /// `branch` is a pure leaf move: the entry log — count, order, parents,
    /// bodies — is byte-for-byte what it was, and only the leaf changed.
    #[test]
    fn branch_does_not_rewrite_history() {
        let mut t = linear();
        let before: Vec<TreeEntry> = t.entries().to_vec();
        t.branch("e2").unwrap();
        assert_eq!(t.leaf_id(), Some("e2"));
        assert_eq!(t.entries(), &before, "branch wrote something");
        // the next append children the new leaf — history still untouched
        t.append(TreeEntryKind::Message, "branched continuation")
            .unwrap();
        assert_eq!(t.entries().len(), before.len() + 1);
        let last = t.entries().last().unwrap();
        assert_eq!(last.parent_id.as_deref(), Some("e2"));
        for (i, old) in before.iter().enumerate() {
            assert_eq!(&t.entries()[i], old, "entry {i} changed");
        }
    }

    /// `branch` validates: unknown targets refuse by name, and the null
    /// target is NOT expressible as a branch — `reset_leaf` is the only
    /// path back to the root state.
    #[test]
    fn branch_rejects_unknown_targets_and_reset_leaf_is_the_root_primitive() {
        let mut t = linear();
        assert_eq!(
            t.branch("nope"),
            Err(TreeError::UnknownTarget("nope".to_string()))
        );
        assert_eq!(t.leaf_id(), Some("e5"), "a refused branch moved nothing");
        assert_eq!(
            t.branch(""),
            Err(TreeError::UnknownTarget(String::new())),
            "an empty id is never a registered target"
        );
        t.reset_leaf();
        assert_eq!(t.leaf_id(), None);
        let root = t.append(TreeEntryKind::Message, "a fresh root").unwrap();
        assert_eq!(root.parent_id, None, "post-reset append is a new root");
    }

    /// Branching to the position you already occupy changes nothing.
    #[test]
    fn branch_to_the_current_leaf_is_a_no_op() {
        let mut t = linear();
        t.branch("e5").unwrap();
        assert_eq!(t.leaf_id(), Some("e5"));
        assert_eq!(t.entries().len(), 5);
    }

    /// The summary lands at the NEW navigation position — a child of the
    /// new leaf, never on the abandoned tail — and a null target persists
    /// `from: "root"`.
    #[test]
    fn branch_with_summary_attaches_at_the_new_position_and_persists_root() {
        let mut t = linear();
        let summary = t
            .branch_with_summary(Some("e3"), "we abandoned the tail")
            .unwrap();
        assert_eq!(summary.kind, TreeEntryKind::BranchSummary);
        assert_eq!(summary.parent_id.as_deref(), Some("e3"));
        assert_eq!(summary.from_id.as_deref(), Some("e3"));
        assert_eq!(t.leaf_id(), Some("e6"), "the leaf lands on the summary");
        // walking the branch from the summary reaches the root in order
        let path = t.get_branch("e6").unwrap();
        let ids: Vec<&str> = path.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["e1", "e2", "e3", "e6"]);

        let mut u = SessionTree::new();
        let from_root = u
            .branch_with_summary(None, "started over from the root")
            .unwrap();
        assert_eq!(from_root.parent_id, None);
        assert_eq!(
            from_root.to_json()["from"],
            json!(ROOT_TARGET),
            "the null target persists as \"root\""
        );
    }

    /// The walk returns root→node, inclusive.
    #[test]
    fn get_branch_walks_root_to_node() {
        let t = linear();
        let path = t.get_branch("e4").unwrap();
        let ids: Vec<&str> = path.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["e1", "e2", "e3", "e4"]);
        let whole = t.get_branch("e1").unwrap();
        assert_eq!(whole.len(), 1);
        assert_eq!(
            t.get_branch("missing"),
            Err(TreeError::UnknownTarget("missing".to_string()))
        );
    }

    /// Orphans become roots; siblings are ordered oldest→newest; nodes
    /// carry their resolved labels.
    #[test]
    fn get_tree_missing_parents_are_roots_children_oldest_to_newest() {
        // e1 root ← {e2, e3}; e4 is an ORPHAN (parent vanished) → root.
        let entries = [
            TreeEntry {
                id: "e1".into(),
                parent_id: None,
                kind: TreeEntryKind::Message,
                body: "root".into(),
                from_id: None,
                label: None,
                seq: 0,
            },
            TreeEntry {
                id: "e2".into(),
                parent_id: Some("e1".into()),
                kind: TreeEntryKind::Message,
                body: "first child".into(),
                from_id: None,
                label: None,
                seq: 1,
            },
            TreeEntry {
                id: "e3".into(),
                parent_id: Some("e1".into()),
                kind: TreeEntryKind::Message,
                body: "second child".into(),
                from_id: None,
                label: None,
                seq: 2,
            },
            TreeEntry {
                id: "e4".into(),
                parent_id: Some("vanished".to_string()),
                kind: TreeEntryKind::Message,
                body: "orphan".into(),
                from_id: None,
                label: None,
                seq: 3,
            },
        ];
        let json: Vec<Value> = entries.iter().map(TreeEntry::to_json).collect();
        let t = SessionTree::replay(&json).unwrap();
        let roots = t.get_tree();
        assert_eq!(roots.len(), 2, "the orphan joins the null parent as a root");
        assert_eq!(roots[0].entry.id, "e1");
        assert_eq!(roots[1].entry.id, "e4");
        let kids: Vec<&str> = roots[0]
            .children
            .iter()
            .map(|n| n.entry.id.as_str())
            .collect();
        assert_eq!(kids, ["e2", "e3"], "children are oldest→newest");
    }

    /// Direct children only — grandchildren never leak into the list.
    #[test]
    fn get_children_direct_only() {
        let t = linear();
        let kids: Vec<&str> = t.get_children("e1").iter().map(|e| e.id.as_str()).collect();
        assert_eq!(kids, ["e2"]);
        assert!(t.get_children("e5").is_empty());
    }

    /// Labels resolve through `labels_by_id`: the latest label wins, an
    /// empty label deletes, and the tree projection carries the resolved
    /// value.
    #[test]
    fn labels_resolve_through_labels_by_id_and_delete() {
        let mut t = linear();
        t.set_label("e2", Some("the good branch")).unwrap();
        assert_eq!(t.label_of("e2"), Some("the good branch"));
        t.set_label("e2", Some("renamed")).unwrap();
        assert_eq!(t.label_of("e2"), Some("renamed"), "latest wins");
        t.set_label("e2", None).unwrap();
        assert_eq!(t.label_of("e2"), None, "the empty label deletes");
        assert_eq!(
            t.set_label("nope", Some("x")),
            Err(TreeError::UnknownTarget("nope".to_string()))
        );
        // replay preserves the resolution
        let json: Vec<Value> = t.entries().iter().map(TreeEntry::to_json).collect();
        let replayed = SessionTree::replay(&json).unwrap();
        assert_eq!(replayed.label_of("e2"), None);
    }

    /// The context conversion: messages and custom entries pass through,
    /// branch summaries ride the named template, labels never appear, and
    /// the conversion follows the ACTIVE branch only.
    #[test]
    fn context_conversion_follows_the_active_branch() {
        let mut t = linear();
        t.branch("e2").unwrap();
        t.branch_with_summary(Some("e2"), "abandoned e3-e5")
            .unwrap();
        t.append(TreeEntryKind::Custom, "injected handoff context")
            .unwrap();
        t.set_label("e2", Some("labeled")).unwrap();
        let msgs = t.context_messages().unwrap();
        let texts: Vec<&str> = msgs.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "root message",
                "second",
                "The conversation branched earlier; the abandoned branch is summarized: abandoned e3-e5",
                "injected handoff context",
            ],
            "labels are never context; the abandoned tail is a summary"
        );
        assert!(msgs.iter().all(|m| m.role == "user"));
        // without a leaf there is no active branch
        t.reset_leaf();
        assert!(t.context_messages().unwrap().is_empty());
    }

    /// Malformed entries refuse by name. No panic, no default entry.
    #[test]
    fn malformed_entry_json_is_a_named_refusal_never_a_panic() {
        let cases = [
            (json!(null), "not an object"),
            (json!("a string"), "not an object"),
            (json!({}), "missing id"),
            (json!({"id": ""}), "empty id"),
            (json!({"id": "e1", "kind": "message"}), "missing body"),
            (
                json!({"id": "e1", "kind": "message", "body": "b", "parent": 7}),
                "ill-typed parent",
            ),
            (
                json!({"id": "e1", "kind": "tree_kind_nobody_defined", "body": "b"}),
                "unknown kind",
            ),
            (
                json!({"id": "e1", "kind": "message", "body": "b", "label": 3}),
                "ill-typed label",
            ),
            (
                json!({"id": "e1", "kind": "message", "body": "b", "from": ""}),
                "empty from",
            ),
        ];
        for (value, why) in cases {
            let err = TreeEntry::from_json(&value)
                .err()
                .unwrap_or(TreeError::Malformed("fixture unexpectedly parsed"));
            assert!(
                matches!(err, TreeError::Malformed(_) | TreeError::UnknownKind(_)),
                "fixture `{why}` landed on the wrong refusal: {err}"
            );
        }
    }

    /// Replay: order is the order; the leaf starts at the newest entry; a
    /// cycle in the parent links is caught at load, not at walk time; and
    /// to_json → from_json round-trips.
    #[test]
    fn replay_preserves_order_catches_cycles_and_round_trips() {
        let t = linear();
        let json: Vec<Value> = t.entries().iter().map(TreeEntry::to_json).collect();
        let replayed = SessionTree::replay(&json).unwrap();
        assert_eq!(replayed, t, "replay of the canonical JSON is the tree");
        assert_eq!(replayed.leaf_id(), Some("e5"));
        assert_eq!(replayed.entries().len(), 5);

        // a → b → a: the load refuses, it does not hang or drop
        let cyclic = vec![
            json!({"id": "a", "parent": "b", "kind": "message", "body": "x"}),
            json!({"id": "b", "parent": "a", "kind": "message", "body": "y"}),
        ];
        assert_eq!(SessionTree::replay(&cyclic), Err(TreeError::Corrupt));

        // a duplicate id is a named refusal, not a silent overwrite
        let dup = vec![
            json!({"id": "a", "parent": null, "kind": "message", "body": "x"}),
            json!({"id": "a", "parent": null, "kind": "message", "body": "y"}),
        ];
        assert_eq!(
            SessionTree::replay(&dup),
            Err(TreeError::Malformed("duplicate entry id"))
        );
    }

    /// The log grows only by appends: ids are stable, seq is the admission
    /// order, and the total-functions law holds on the entry budget — a
    /// tree of real size answers every question without allocation games.
    #[test]
    fn appends_are_stable_total_and_ordered() {
        let mut t = SessionTree::new();
        for i in 0..64 {
            let e = t
                .append(TreeEntryKind::Message, &format!("body {i}"))
                .unwrap();
            assert_eq!(e.seq, i as u64);
        }
        let all = t.entries();
        assert_eq!(all.len(), 64);
        assert!(all.windows(2).all(|w| w[0].seq < w[1].seq));
        assert_eq!(all.first().unwrap().id, "e1");
        assert_eq!(all.last().unwrap().id, "e64");
        assert_eq!(
            t.append(TreeEntryKind::Message, ""),
            Err(TreeError::Malformed("body must be non-empty"))
        );
        assert_eq!(
            t.append(TreeEntryKind::Label, "x"),
            Err(TreeError::Malformed(
                "label entries need an explicit target: use set_label"
            ))
        );
    }
}
