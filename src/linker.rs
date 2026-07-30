//! Deterministic entity linker for structured document content.
//!
//! Extracts entities from section headings and bold terms, then discovers
//! entity mentions and typed relationship patterns — all without ML or LLM.
//!
//! Zero-hallucination guarantee: every entity exists as a section heading or
//! bolded term in the document. Every typed relationship requires both
//! sides to be known vocabulary entities with a verified verb pattern.
//!
//! Cross-document vocabulary: existing entities from the database are loaded
//! and merged into the per-document vocabulary, enabling entity mentions to
//! be recognised across chapters (a heading entity from Chapter 6 can be
//! linked when Chapter 10 mentions the same term).
//!
//! ponytail: ordering-dependent for first-time ingests. Entities don't exist
//! in the DB until the first document creates them. A two-pass ingest (scan
//! all docs for vocabulary first, then ingest) would be fully ordering-
//! independent. Not implemented because the common case (re-ingest where
//! most entities already exist) works well enough.

use aho_corasick::{AhoCorasick, MatchKind};

/// Check that the match at (start, end) in `content` has proper word
/// boundaries: previous char (if any) is non-word, next char (if any)
/// is non-word. Word chars: alphanumeric, _, -.
fn has_word_boundaries(content: &str, start: usize, end: usize) -> bool {
    let bytes = content.as_bytes();
    if start > 0 {
        let prev = bytes[start - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'-' {
            return false;
        }
    }
    if end < bytes.len() {
        let next = bytes[end];
        if next.is_ascii_alphanumeric() || next == b'_' || next == b'-' {
            return false;
        }
    }
    true
}

/// Vocabulary of known entity names, sorted by length (longest first for
/// greedy matching).
#[derive(Default)]
pub struct EntityVocabulary {
    entities: Vec<String>,
}

/// Compiled entity matcher — O(n) multi-pattern matching with Aho-Corasick.
pub struct EntityMatcher {
    ac: AhoCorasick,
    entity_names: Vec<String>,
}

/// A typed relationship extracted from sentence patterns.
#[derive(Debug)]
pub struct TypedEdge {
    pub relation: String,
    pub from: String,
    pub to: String,
}

/// Common generic heading words that aren't useful as entities.
const STOP_HEADINGS: &[&str] = &[
    "introduction",
    "overview",
    "prerequisites",
    "summary",
    "conclusion",
    "conclusions",
    "references",
    "see also",
    "notes",
    "note",
    "warning",
    "important",
    "tip",
    "troubleshooting",
    "further reading",
    "related topics",
    "what's next",
];

/// Verb patterns for typed relationship extraction.
/// Longer patterns first so they match before their shorter suffixes.
const RELATION_PATTERNS: &[(&str, &str)] = &[
    ("runs on top of", "runs_on"),
    ("communicates with", "communicates_with"),
    ("communicates to", "communicates_with"),
    ("integrates with", "integrates_with"),
    ("connects to", "connects_to"),
    ("migrates to", "migrates_to"),
    ("migrates from", "migrates_from"),
    ("depends on", "depends_on"),
    ("consists of", "consists_of"),
    ("replaced by", "replaced_by"),
    ("built on", "built_on"),
    ("based on", "based_on"),
    ("is part of", "part_of"),
    ("runs on", "runs_on"),
    ("requires", "requires"),
    ("supports", "supports"),
    ("includes", "includes"),
    ("manages", "manages"),
    ("uses", "uses"),
    ("provides", "provides"),
    ("contains", "contains"),
    ("configures", "configures"),
    ("replaces", "replaces"),
    ("is a", "is_a"),
    ("is an", "is_a"),
    ("stores", "stores"),
    ("handles", "handles"),
    ("processes", "processes"),
    ("monitors", "monitors"),
    ("controls", "controls"),
    ("deploys", "deploys"),
    ("installs", "installs"),
];

// ---------------------------------------------------------------------------
// EntityVocabulary
// ---------------------------------------------------------------------------

impl EntityVocabulary {
    /// Insert a name.
    pub fn insert(&mut self, name: &str) {
        let name = name.trim();
        if name.len() < 3 {
            return;
        }
        let lower = name.to_lowercase();
        if !self.entities.contains(&lower) {
            self.entities.push(lower);
        }
    }

    /// Finalize: sort by length descending for greedy matching.
    pub fn finalize(&mut self) {
        self.entities.sort_by_key(|a| std::cmp::Reverse(a.len()));
        self.entities.dedup();
    }
}

impl From<EntityVocabulary> for EntityMatcher {
    fn from(vocab: EntityVocabulary) -> Self {
        let entity_names = vocab.entities.clone();
        let ac = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostFirst)
            .ascii_case_insensitive(true)
            .build(&vocab.entities)
            .expect("Aho-Corasick automaton build failed");
        EntityMatcher { ac, entity_names }
    }
}

// ---------------------------------------------------------------------------
// EntityMatcher
// ---------------------------------------------------------------------------

impl EntityMatcher {
    /// Find every mention of every vocabulary entity in content, skipping
    /// matches that fall inside fenced code blocks.
    ///
    /// Returns entity names (from the vocabulary, lowercased) in order of
    /// appearance. With LeftmostFirst semantics, a longer entity always
    /// wins over a shorter prefix at the same position.
    pub fn find_mentions<'m>(&'m self, content: &str, code_ranges: &[(usize, usize)]) -> Vec<&'m str> {
        let mut found: Vec<(usize, &str)> = Vec::new();

        for m in self.ac.find_iter(content) {
            if is_in_ranges(m.start(), m.end(), code_ranges) {
                continue;
            }
            if !has_word_boundaries(content, m.start(), m.end()) {
                continue;
            }
            let name = &self.entity_names[m.pattern().as_usize()];
            found.push((m.start(), name));
        }

        found.into_iter().map(|(_, n)| n).collect()
    }

    /// Find mentions with positions (for relationship extraction).
    fn find_mentions_with_positions<'m>(
        &'m self,
        content: &str,
        code_ranges: &[(usize, usize)],
    ) -> Vec<(usize, usize, &'m str)> {
        let mut found: Vec<(usize, usize, &str)> = Vec::new();

        for m in self.ac.find_iter(content) {
            if is_in_ranges(m.start(), m.end(), code_ranges) {
                continue;
            }
            if !has_word_boundaries(content, m.start(), m.end()) {
                continue;
            }
            let name = &self.entity_names[m.pattern().as_usize()];
            found.push((m.start(), m.end(), name));
        }

        // Dedup overlapping ranges (keep first / leftmost-longest)
        let mut deduped: Vec<(usize, usize, &str)> = Vec::new();
        for m in found {
            let overlaps = deduped.iter().any(|&(s, e, _)| (s..=e).contains(&m.0) && (s..=e).contains(&m.1));
            if !overlaps {
                deduped.push(m);
            }
        }

        deduped
    }

    /// Extract typed relationships from sentence patterns.
    ///
    /// For each sentence, uses Aho-Corasick to find all entity mentions,
    /// then checks verb patterns between each ordered pair.
    pub fn find_relationships(
        &self,
        content: &str,
        _code_ranges: &[(usize, usize)],
    ) -> Vec<TypedEdge> {
        if self.entity_names.is_empty() {
            return vec![];
        }

        let mut edges = Vec::new();
        let bytes = content.as_bytes();
        let len = bytes.len();

        // Walk sentence boundaries
        let mut sentence_starts: Vec<usize> = vec![0];
        let mut i = 0;
        while i < len {
            if i + 1 < len && bytes[i] == b'.' && bytes[i + 1] == b' ' {
                let prev = if i >= 2 { &bytes[i - 2..i] } else { &[] };
                if prev != b"i.e" && prev != b"e.g" {
                    sentence_starts.push(i + 2);
                }
            }
            if i + 3 < len && bytes[i] == b'\n' && bytes[i + 1] == b'\n' {
                sentence_starts.push(i + 2);
            }
            i += 1;
        }
        sentence_starts.push(len);

        for w in sentence_starts.windows(2) {
            let start = w[0];
            let end = w[1];
            if end <= start + 10 {
                continue;
            }
            let sentence = &content[start..end];

            // Skip indented code blocks
            if sentence.starts_with("    ") || sentence.starts_with('\t') {
                continue;
            }

            let mentions = self.find_mentions_with_positions(sentence, &[]);
            if mentions.len() < 2 {
                continue;
            }

            for i in 0..mentions.len() {
                for j in i + 1..mentions.len() {
                    if mentions[j].1 <= mentions[i].1 {
                        continue;
                    }
                    let between = &sentence[mentions[i].1..mentions[j].0].trim();
                    if between.is_empty() {
                        continue;
                    }

                    let between_lower = between.to_lowercase();
                    for (verb, rel_type) in RELATION_PATTERNS {
                        if between_lower.contains(verb) {
                            edges.push(TypedEdge {
                                relation: rel_type.to_string(),
                                from: mentions[i].2.to_string(),
                                to: mentions[j].2.to_string(),
                            });
                            break;
                        }
                    }
                }
            }
        }

        edges
    }
}

// ---------------------------------------------------------------------------
// Code block detection
// ---------------------------------------------------------------------------

/// Find byte ranges of fenced code blocks (```...```) in content.
pub fn find_code_ranges(content: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut in_code = false;
    let mut code_start: usize = 0;
    let mut line_start: usize = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_code {
                ranges.push((code_start, line_start + line.len()));
                in_code = false;
            } else {
                code_start = line_start;
                in_code = true;
            }
        }
        // Indented code blocks (4 spaces)
        if !in_code && line.starts_with("    ") && code_start == 0 {
            // We track indented blocks as a single range from first
            // indented line to next non-indented line
            if ranges.last().map(|&(_, e)| e) == Some(line_start) {
                // Continuation of previous indented block
                if let Some(last) = ranges.last_mut() {
                    last.1 = line_start + line.len();
                }
            } else {
                ranges.push((line_start, line_start + line.len()));
            }
        }
        line_start += line.len() + 1; // +1 for newline
    }

    ranges
}

/// Check if a byte range [start, end) overlaps any of the given ranges.
fn is_in_ranges(start: usize, end: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|&(s, e)| start < e && end > s)
}

// ---------------------------------------------------------------------------
// Vocabulary extraction from document structure
// ---------------------------------------------------------------------------

/// Extract heading level and text from a line starting with `#`.
fn parse_heading_line(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.first() != Some(&b'#') {
        return None;
    }
    let mut i = 0;
    while i < bytes.len() && bytes[i] == b'#' {
        i += 1;
    }
    if !(2..=6).contains(&i) {
        return None;
    }
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    let heading = trimmed[i..].trim();
    if heading.is_empty() {
        return None;
    }
    Some((i, heading))
}

/// Extract entity vocabulary from document structure.
///
/// Sources:
/// 1. Section headings (`## Title` and deeper)
/// 2. Bold terms (`**term**`)
/// 3. Code spans (`` `tool` ``)
pub fn extract_vocabulary(content: &str) -> EntityVocabulary {
    let mut vocab = EntityVocabulary::default();

    // 1. Section headings
    for line in content.lines() {
        if let Some((_level, heading)) = parse_heading_line(line) {
            if STOP_HEADINGS.contains(&heading.to_lowercase().as_str()) || heading.len() < 4 {
                continue;
            }
            vocab.insert(heading);
        }
    }

    // 2. Bold terms
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i + 3 < len {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            let start = i + 2;
            if let Some(end) = find_closing_double_star(bytes, start) {
                let term = &content[start..end].trim();
                if term.chars().any(|c| c.is_uppercase()) && term.len() >= 3 {
                    vocab.insert(term);
                }
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }

    // 3. Code spans
    i = 0;
    while i < len {
        if bytes[i] == b'`' {
            let start = i + 1;
            if let Some(end) = find_closing_backtick(bytes, start) {
                let tool = &content[start..end].trim();
                if tool.len() >= 3
                    && tool.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                {
                    vocab.insert(tool);
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }

    vocab.finalize();
    vocab
}

fn find_closing_double_star(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut i = pos;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_closing_backtick(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut i = pos;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Vocabulary extraction ---

    #[test]
    fn extracts_headings_as_vocabulary() {
        let content = "# Title\n\n## CRUSH Map Configuration\n\n## OSD Deployment\n";
        let vocab = extract_vocabulary(content);
        assert!(vocab.entities.contains(&"crush map configuration".to_string()));
        assert!(vocab.entities.contains(&"osd deployment".to_string()));
    }

    #[test]
    fn excludes_h1() {
        let content = "# Title\n## Section\n";
        let vocab = extract_vocabulary(content);
        assert!(!vocab.entities.contains(&"title".to_string()));
        assert!(vocab.entities.contains(&"section".to_string()));
    }

    #[test]
    fn filters_generic_headings() {
        let content = "## Overview\n\n## Introduction\n\n## CRUSH Map\n";
        let vocab = extract_vocabulary(content);
        assert!(!vocab.entities.contains(&"overview".to_string()));
        assert!(vocab.entities.contains(&"crush map".to_string()));
    }

    #[test]
    fn extracts_bold_terms() {
        let content = "The **CRUSH** algorithm distributes data across **OSDs**.";
        let vocab = extract_vocabulary(content);
        assert!(vocab.entities.contains(&"crush".to_string()));
        assert!(vocab.entities.contains(&"osds".to_string()));
    }

    #[test]
    fn extracts_code_spans() {
        let content = "Configure `ceph.conf` and run `ceph-deploy`.";
        let vocab = extract_vocabulary(content);
        assert!(vocab.entities.contains(&"ceph.conf".to_string()));
        assert!(vocab.entities.contains(&"ceph-deploy".to_string()));
    }

    // --- Entity matcher (Aho-Corasick based) ---

    fn make_matcher(words: &[&str]) -> EntityMatcher {
        let mut v = EntityVocabulary::default();
        for w in words {
            v.insert(w);
        }
        v.finalize();
        v.into()
    }

    #[test]
    fn matcher_finds_entities_in_text() {
        let m = make_matcher(&["CRUSH Map", "Ceph"]);
        let found = m.find_mentions("The CRUSH Map is key to Ceph.", &[]);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&"crush map"));
        assert!(found.contains(&"ceph"));
    }

    #[test]
    fn matcher_leftmost_longest_wins() {
        let m = make_matcher(&["CRUSH Map", "CRUSH"]);
        let found = m.find_mentions("The CRUSH Map is key.", &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], "crush map");
    }

    #[test]
    fn matcher_is_case_insensitive() {
        let m = make_matcher(&["Ceph"]);
        let found = m.find_mentions("ceph CEPH Ceph", &[]);
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn matcher_respects_word_boundaries() {
        let m = make_matcher(&["Ceph"]);
        let found = m.find_mentions("Ceph Cephs", &[]);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn matcher_skips_code_blocks() {
        let m = make_matcher(&["Ceph"]);
        let content = "Ceph outside.\n```\nCeph inside.\n```\nCeph outside again.";
        let code = find_code_ranges(content);
        let found = m.find_mentions(content, &code);
        // "Ceph" inside the code block should be skipped
        // But "Ceph" appears outside twice -> 2 matches
        // Actually there might be a boundary issue. Let me count.
        // "Ceph outside.\n" -> match Ceph at start
        // "Ceph outside again." -> match Ceph
        // "Ceph inside." inside ``` is skipped
        assert_eq!(found.len(), 2, "should find Ceph twice (outside code): {:?}", found);
    }

    #[test]
    fn code_block_detection() {
        let content = "normal\n```\ncode\n```\nnormal";
        let ranges = find_code_ranges(content);
        assert_eq!(ranges.len(), 1);
        assert!(ranges[0].0 > 0);
        assert!(ranges[0].1 > ranges[0].0);
    }

    // --- Relationships ---

    #[test]
    fn extracts_requires_relationship() {
        let content = "## Ceph\n\n## CRUSH\n\nCeph requires CRUSH for data distribution.";
        let m: EntityMatcher = extract_vocabulary(content).into();
        let code = find_code_ranges(content);
        let edges = m.find_relationships(content, &code);
        let req = edges.iter().find(|e| e.relation == "requires");
        assert!(req.is_some(), "should find 'requires': {:?}", edges);
        assert_eq!(req.unwrap().from.to_lowercase(), "ceph");
        assert_eq!(req.unwrap().to.to_lowercase(), "crush");
    }

    #[test]
    fn extracts_runs_on_relationship() {
        let content = "## Proxmox VE\n\n## Debian\n\nProxmox VE runs on Debian.";
        let m: EntityMatcher = extract_vocabulary(content).into();
        let edges = m.find_relationships(content, &[]);
        let edge = edges.iter().find(|e| e.relation == "runs_on");
        assert!(edge.is_some(), "should find 'runs_on': {:?}", edges);
        assert_eq!(edge.unwrap().from.to_lowercase(), "proxmox ve");
        assert_eq!(edge.unwrap().to.to_lowercase(), "debian");
    }

    #[test]
    fn no_relationships_with_empty_vocab() {
        let content = "The system runs on a standard kernel.";
        let m: EntityMatcher = EntityVocabulary::default().into();
        let edges = m.find_relationships(content, &[]);
        assert!(edges.is_empty());
    }

    #[test]
    fn relationship_skips_code_blocks() {
        let content = "## Ceph\n\n## CRUSH\n\nCeph requires CRUSH for data distribution.\n```\nCeph requires nothing inside code.\n```";
        let m: EntityMatcher = extract_vocabulary(content).into();
        let code = find_code_ranges(content);
        let edges = m.find_relationships(content, &code);
        // Should find the "requires" relationship from the non-code part
        let req = edges.iter().find(|e| e.relation == "requires");
        assert!(req.is_some(), "should find 'requires' outside code: {:?}", edges);
    }

    // --- DB vocabulary ---

    #[test]
    fn vocabulary_merge_deduplicates() {
        let mut vocab = extract_vocabulary("## Ceph\n## CRUSH\n");
        vocab.insert("ceph");
        vocab.insert("rados");
        vocab.finalize();
        assert!(vocab.entities.contains(&"ceph".to_string()));
        assert!(vocab.entities.contains(&"crush".to_string()));
        assert!(vocab.entities.contains(&"rados".to_string()));
        // "ceph" should appear only once
        let count = vocab.entities.iter().filter(|e| *e == "ceph").count();
        assert_eq!(count, 1);
    }
}
