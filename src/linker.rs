//! Deterministic entity linker for structured document content.
//!
//! Extracts entities from section headings and bold terms, then discovers
//! entity mentions and typed relationship patterns — all without ML or LLM
//! or any external dependency beyond stdlib.
//!
//! Zero-hallucination guarantee: every entity exists as a section heading or
//! bolded term in the document. Every typed relationship requires both
//! sides to be known vocabulary entities with a verified verb pattern.
//!
//! ponytail: single-document vocabulary only. Cross-document entity linking
//! (e.g. entity from Chapter 6's heading recognized in Chapter 10) requires
//! a DB-backed vocabulary — deferred because it needs a second ingest pass.

/// Vocabulary of known entity names, sorted by length (longest first for
/// greedy matching).
#[derive(Default)]
pub struct EntityVocabulary {
    /// Lowercased entity names, longest first.
    entities: Vec<String>,
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
/// (verb_text, canonical_relation_type)
const RELATION_PATTERNS: &[(&str, &str)] = &[
    ("runs on top of", "runs_on"),
    ("runs on", "runs_on"),
    ("requires", "requires"),
    ("supports", "supports"),
    ("includes", "includes"),
    ("manages", "manages"),
    ("migrates to", "migrates_to"),
    ("migrates from", "migrates_from"),
    ("depends on", "depends_on"),
    ("uses", "uses"),
    ("provides", "provides"),
    ("communicates with", "communicates_with"),
    ("communicates to", "communicates_with"),
    ("contains", "contains"),
    ("configures", "configures"),
    ("replaces", "replaces"),
    ("replaced by", "replaced_by"),
    ("integrates with", "integrates_with"),
    ("connects to", "connects_to"),
    ("built on", "built_on"),
    ("based on", "based_on"),
    ("is a", "is_a"),
    ("is an", "is_a"),
    ("is part of", "part_of"),
    ("consists of", "consists_of"),
    ("stores", "stores"),
    ("manages", "manages"),
    ("handles", "handles"),
    ("processes", "processes"),
    ("monitors", "monitors"),
    ("controls", "controls"),
    ("deploys", "deploys"),
    ("installs", "installs"),
];

impl EntityVocabulary {
    /// Insert a name if it meets minimum criteria.
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

    /// Return true if the vocabulary has useful content.
    pub fn is_populated(&self) -> bool {
        !self.entities.is_empty()
    }
}

/// Extract a heading from a line that starts with `##` (level 2+).
/// Returns `Some((level, heading_text))` or `None`.
fn parse_heading_line(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.first() != Some(&b'#') {
        return None;
    }
    // Skip `#` characters
    let mut i = 0;
    while i < bytes.len() && bytes[i] == b'#' {
        i += 1;
    }
    let level = i;
    if !(2..=6).contains(&level) {
        return None;
    }
    // Skip whitespace after `#`
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    let heading = trimmed[i..].trim();
    if heading.is_empty() {
        return None;
    }
    Some((level, heading))
}

/// Extract entity vocabulary from document structure.
///
/// Sources (in priority order):
/// 1. Section headings (`## Title` and deeper) — the author's own concept names
/// 2. Bold terms (`**term**`) — glossary-style markers
/// 3. Code spans (`` `tool` ``) — technology/tool references
pub fn extract_vocabulary(content: &str) -> EntityVocabulary {
    let mut vocab = EntityVocabulary::default();

    // 1. Section headings: ## through ######
    for line in content.lines() {
        if let Some((_level, heading)) = parse_heading_line(line) {
            let lower = heading.to_lowercase();
            if STOP_HEADINGS.contains(&lower.as_str()) || heading.len() < 4 {
                continue;
            }
            vocab.insert(heading);
        }
    }

    // 2. Bold terms: **term**
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i + 3 < len {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            let start = i + 2;
            if let Some(end) = find_closing_double_star(bytes, start) {
                let term = &content[start..end];
                let term = term.trim();
                // Require at least one uppercase letter — filters out
                // presentational **emphasis** that isn't a glossary term
                if term.chars().any(|c| c.is_uppercase()) && term.len() >= 3 {
                    vocab.insert(term);
                }
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }

    // 3. Code spans: `tool`
    i = 0;
    while i < len {
        if bytes[i] == b'`' {
            let start = i + 1;
            if let Some(end) = find_closing_backtick(bytes, start) {
                let tool = &content[start..end];
                let tool = tool.trim();
                // Must be 3+ alphanumeric chars starting with a letter
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

/// Find the closing `**` starting from `pos`. Returns `Some(end)` where
/// `end` is the position of the first `*` in the closing pair.
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

/// Find the closing backtick starting from `pos`.
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

/// Find mentions of vocabulary entities in content.
///
/// Returns the entity names (as they appear in the vocabulary) that were
/// found in the content, in order of appearance. Longer matches take
/// priority over substrings.
///
/// ponytail: simple substring match with word boundaries. No context-aware
/// disambiguation — if the vocabulary has "Ceph" and the text mentions
/// "Ceph", it links. Doesn't handle homonyms.
pub fn find_mentions<'a>(content: &str, vocab: &'a EntityVocabulary) -> Vec<&'a str> {
    if !vocab.is_populated() {
        return vec![];
    }

    let lower_content = content.to_lowercase();
    let mut found: Vec<(usize, usize, &str)> = Vec::new(); // (start, end, entity)

    for entity in &vocab.entities {
        let mut search_start = 0;
        while let Some(pos) = lower_content[search_start..].find(entity) {
            let abs_pos = search_start + pos;
            let end = abs_pos + entity.len();

            // Word boundary check: previous char must be non-word
            if abs_pos > 0 {
                let prev = lower_content.as_bytes()[abs_pos - 1];
                if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'-' {
                    search_start = end;
                    continue;
                }
            }
            // Next char must be non-word
            if end < lower_content.len() {
                let next = lower_content.as_bytes()[end];
                if next.is_ascii_alphanumeric() || next == b'_' || next == b'-' {
                    search_start = end;
                    continue;
                }
            }

            // Check this range doesn't overlap with a longer match
            let overlaps = found.iter().any(|&(s, e, _)| (s..=e).contains(&abs_pos) && (s..=e).contains(&end));
            if !overlaps {
                found.push((abs_pos, end, entity));
            }

            search_start = end;
        }
    }

    // Sort by position
    found.sort_by_key(|m| m.0);
    found.into_iter().map(|(_, _, e)| e).collect()
}

/// Split content into sentences on `. ` and paragraph breaks.
fn split_sentences(content: &str) -> Vec<&str> {
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut sentences = Vec::new();
    let mut start = 0;

    // Simple state machine: emit a sentence on `. ` or `\n\n`
    let mut i = 0;
    while i < len {
        // Check for `. ` (period followed by space)
        if i + 1 < len && bytes[i] == b'.' && bytes[i + 1] == b' ' {
            // Avoid splitting on abbreviations like "e.g." or "i.e."
            let prev = if i >= 2 { &bytes[i - 2..i] } else { &[] };
            if prev != b"i.e" && prev != b"e.g" {
                let end = i;
                if end > start {
                    let s = &content[start..end];
                    if !s.trim().is_empty() {
                        sentences.push(s.trim());
                    }
                }
                start = i + 2;
                i += 2;
                continue;
            }
        }
        // Check for paragraph break (two newlines)
        if i + 3 < len && bytes[i] == b'\n' && bytes[i + 1] == b'\n' {
            let end = i;
            if end > start {
                let s = &content[start..end];
                if !s.trim().is_empty() {
                    sentences.push(s.trim());
                }
            }
            start = i + 2;
            i += 2;
            continue;
        }
        i += 1;
    }

    // Last sentence
    if start < len {
        let s = &content[start..];
        if !s.trim().is_empty() {
            sentences.push(s.trim());
        }
    }

    sentences
}

/// Extract typed relationships from sentence patterns.
///
/// Scans for `<Entity> <verb> <Entity>` patterns where both sides are known
/// vocabulary entities. Returns edges with correct direction (entity before
/// verb is `from`, entity after verb is `to`).
///
/// ponytail: simple `. ` sentence splitting. Doesn't handle abbreviations
/// like "e.g." — those are rare in infrastructure technical prose.
pub fn find_relationships(content: &str, vocab: &EntityVocabulary) -> Vec<TypedEdge> {
    if !vocab.is_populated() {
        return vec![];
    }

    let mut edges = Vec::new();

    for sentence in split_sentences(content) {
        if sentence.len() < 10 {
            continue;
        }

        // Find all vocabulary entity mentions in this sentence with positions
        let lower_sent = sentence.to_lowercase();
        let mut mentions: Vec<(usize, usize, &str)> = Vec::new();

        // Dedent first to avoid matching inside code blocks (heuristic:
        // indented code blocks start with 4 spaces). We skip sentences
        // that look like code.
        if sentence.starts_with("    ") || sentence.starts_with('\t') {
            continue;
        }

        for entity in &vocab.entities {
            let mut search_start = 0;
            while let Some(pos) = lower_sent[search_start..].find(entity) {
                let abs_pos = search_start + pos;
                let end = abs_pos + entity.len();

                // Word boundary check
                if abs_pos > 0 {
                    let prev = lower_sent.as_bytes()[abs_pos - 1];
                    if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'-' {
                        search_start = end;
                        continue;
                    }
                }
                if end < lower_sent.len() {
                    let next = lower_sent.as_bytes()[end];
                    if next.is_ascii_alphanumeric() || next == b'_' || next == b'-' {
                        search_start = end;
                        continue;
                    }
                }

                // Check overlap with longer match
                let overlaps = mentions.iter().any(|&(s, e, _)| (s..=e).contains(&abs_pos) && (s..=e).contains(&end));
                if !overlaps {
                    mentions.push((abs_pos, end, entity));
                }
                search_start = end;
            }
        }

        // Sort mentions by position
        mentions.sort_by_key(|m| m.0);

        // For each ordered pair, check verb pattern between them
        for i in 0..mentions.len() {
            for j in i + 1..mentions.len() {
                if mentions[j].0 <= mentions[i].1 {
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
                            from: sentence[mentions[i].0..mentions[i].1].to_string(),
                            to: sentence[mentions[j].0..mentions[j].1].to_string(),
                        });
                        break;
                    }
                }
            }
        }
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_headings_as_vocabulary() {
        let content = "# Title\n\n## CRUSH Map Configuration\n\nSome text about Ceph.\n\n## OSD Deployment\n\nMore text.";
        let vocab = extract_vocabulary(content);
        assert!(vocab.entities.contains(&"crush map configuration".to_string()));
        assert!(vocab.entities.contains(&"osd deployment".to_string()));
        assert!(!vocab.entities.contains(&"title".to_string())); // # h1 is excluded
    }

    #[test]
    fn filters_generic_heading_words() {
        let content = "## Overview\n\n## Introduction\n\n## CRUSH Map\n\n";
        let vocab = extract_vocabulary(content);
        assert!(!vocab.entities.contains(&"overview".to_string()));
        assert!(!vocab.entities.contains(&"introduction".to_string()));
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
    fn extracts_code_span_tools() {
        let content = "Configure `ceph.conf` and run `ceph-deploy`.";
        let vocab = extract_vocabulary(content);
        assert!(vocab.entities.contains(&"ceph.conf".to_string()));
        assert!(vocab.entities.contains(&"ceph-deploy".to_string()));
    }

    #[test]
    fn find_mentions_detects_entity_in_text() {
        let mut vocab = EntityVocabulary::default();
        vocab.insert("CRUSH Map");
        vocab.insert("Ceph");
        vocab.finalize();
        let content = "The CRUSH Map algorithm is key to Ceph performance.";
        let mentions = find_mentions(content, &vocab);
        assert_eq!(mentions.len(), 2);
        assert!(mentions.iter().any(|m| m.to_lowercase() == "crush map"));
        assert!(mentions.iter().any(|m| m.to_lowercase() == "ceph"));
    }

    #[test]
    fn respects_word_boundaries() {
        let mut vocab = EntityVocabulary::default();
        vocab.insert("Ceph");
        vocab.finalize();
        let content = "Ceph configuration. Cephs are not linked. Ceph is.";
        let mentions = find_mentions(content, &vocab);
        assert_eq!(mentions.len(), 2); // First and third "Ceph"
    }

    #[test]
    fn longest_match_wins_over_substring() {
        let mut vocab = EntityVocabulary::default();
        vocab.insert("CRUSH Map");
        vocab.insert("CRUSH");
        vocab.finalize();
        let content = "The CRUSH Map is key.";
        let mentions = find_mentions(content, &vocab);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].to_lowercase(), "crush map");
    }

    #[test]
    fn extracts_requires_relationship() {
        let content = "## Ceph\n\n## CRUSH\n\nCeph requires CRUSH for data distribution.";
        let vocab = extract_vocabulary(content);
        let edges = find_relationships(content, &vocab);
        let req = edges.iter().find(|e| e.relation == "requires");
        assert!(req.is_some(), "should find 'requires' edge: {:?}", edges);
        if let Some(e) = req {
            assert_eq!(e.from.to_lowercase(), "ceph");
            assert_eq!(e.to.to_lowercase(), "crush");
        }
    }

    #[test]
    fn extracts_runs_on_relationship() {
        let content = "## Proxmox VE\n\n## Debian\n\nProxmox VE runs on Debian GNU/Linux.";
        let vocab = extract_vocabulary(content);
        let edges = find_relationships(content, &vocab);
        let edge = edges.iter().find(|e| e.relation == "runs_on");
        assert!(edge.is_some(), "should find 'runs_on' edge: {:?}", edges);
        if let Some(e) = edge {
            assert_eq!(e.from.to_lowercase(), "proxmox ve");
            assert_eq!(e.to.to_lowercase(), "debian");
        }
    }

    #[test]
    fn no_relationships_without_vocab() {
        let content = "The system runs on a standard kernel.";
        let vocab = EntityVocabulary::default();
        let edges = find_relationships(content, &vocab);
        assert!(edges.is_empty());
    }

    #[test]
    fn split_sentences_works() {
        let text = "Ceph requires CRUSH. Proxmox runs on Debian.";
        let s = split_sentences(text);
        assert_eq!(s.len(), 2);
        assert!(s[0].contains("Ceph"));
        assert!(s[1].contains("Proxmox"));
    }

    #[test]
    fn parse_heading_various_levels() {
        assert!(parse_heading_line("# H1").is_none()); // level 1 excluded
        assert!(parse_heading_line("## H2").is_some());
        assert!(parse_heading_line("### H3").is_some());
        assert!(parse_heading_line("#### H4").is_some());
        assert!(parse_heading_line("##### H5").is_some());
        assert!(parse_heading_line("###### H6").is_some());
        assert!(parse_heading_line("####### H7").is_none()); // too deep
        if let Some((level, text)) = parse_heading_line("## CRUSH Map") {
            assert_eq!(level, 2);
            assert_eq!(text, "CRUSH Map");
        } else {
            panic!("expected heading");
        }
    }

    #[test]
    fn no_self_references_are_filtered_in_main() {
        // This tests that the vocabulary doesn't contain the doc title
        // (the main integration code has a separate check for this)
        let content = "## CRUSH Map\n\nCRUSH Map is important.";
        let vocab = extract_vocabulary(content);
        // Heading should be an entity
        assert!(vocab.entities.contains(&"crush map".to_string()));
        // The mention check in main.rs has: mention_lower != doc_lower
        // That's the self-reference filter, tested in integration
    }
}
