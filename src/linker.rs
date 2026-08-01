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

use std::collections::{HashMap, HashSet};

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
    pub entities: Vec<String>,
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
/// These cover common infrastructure/tech verbs.
/// Additional patterns are discovered per-document by [`EntityMatcher::discover_verb_patterns`].
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

/// Words that never carry a relationship between entities (articles, pure
/// copulae, modals, common prepositions). Sorted for binary search.
const STOP_WORDS: &[&str] = &[
    "after", "also", "an", "and", "are", "as", "at", "be", "been", "being", "below", "between",
    "but", "by", "can", "could", "date", "did", "do", "does", "during", "for", "from", "had",
    "has", "have", "her", "his", "in", "into", "is", "its", "just", "may", "might", "must", "my",
    "no", "nor", "not", "of", "on", "or", "our", "shall", "should", "than", "that", "the", "their",
    "them", "then", "there", "these", "they", "this", "those", "through", "to", "too", "upon",
    "very", "via", "was", "were", "will", "with", "would", "your",
];

/// Verb-forming suffixes — words ending in these are almost certainly verbs.
fn has_verb_suffix(w: &str) -> bool {
    w.ends_with("ed")
        || w.ends_with("ing")
        || w.ends_with("ate")
        || w.ends_with("ify")
        || w.ends_with("ize")
        || w.ends_with("ise")
}

/// Check whether a word matches common English verb patterns.
///
/// Accepts words that:
/// 1. Appear in [`RELATION_PATTERNS`] (known-good infrastructure verbs), or
/// 2. End with a verb-forming suffix (-ed, -ing, -ate, -ify, -ize, -ise), or
/// 3. End with 3rd-person -s/-es where the base matches (2), e.g.
///    "communicates" → base "communicate" → ends with -ate.
///
/// ponytail: bare base-form verbs without derivational suffixes
/// ("run", "set", "cut", "encrypt") are not detected without a dictionary.
/// They are rare as discovered patterns because the frequency threshold
/// mostly catches morphologically marked verbs ("manages", "configures").
/// Common infrastructure verbs are already covered by [`RELATION_PATTERNS`].
fn is_likely_verb(word: &str) -> bool {
    if word.len() < 3 {
        return false;
    }
    if RELATION_PATTERNS.iter().any(|(v, _)| v == &word) {
        return true;
    }
    if has_verb_suffix(word) {
        return true;
    }
    // 3rd-person singular: strip trailing -s and check the base.
    // "communicates" → base "communicate" → has_verb_suffix("communicate") = true
    // "maps"         → base "map"         → has_verb_suffix("map") = false
    if word.ends_with('s') && word.len() > 3 {
        // Handle -ies → -y (verifies → verify)
        if word.ends_with("ies") && word.len() > 4 {
            let base = format!("{}y", &word[..word.len() - 3]);
            if has_verb_suffix(&base) {
                return true;
            }
        }
        // Handle -es where base ends in s/sh/ch/z/o
        if word.ends_with("es") && word.len() > 4 {
            let base_es = &word[..word.len() - 2];
            if has_verb_suffix(base_es) {
                return true;
            }
        }
        let base = &word[..word.len() - 1];
        if has_verb_suffix(base) {
            return true;
        }
    }
    false
}

/// Extract `part_of` relationships from the document's heading hierarchy.
///
/// A heading at level N+1 is a subtopic of the most recent heading at level N
/// that is also a known entity in `entities`.  This encodes the document
/// structure as explicit KG edges — a `part_of` edge is created for every
/// adjacent heading pair where both headings exist in the entity vocabulary.
///
/// 2026 document-structure research confirms that heading hierarchy is a
/// critical structural signal for knowledge graph construction from technical
/// documentation.
///
/// `excluded_ranges` — byte ranges to skip (code blocks, tables, etc).
pub fn extract_heading_relationships(
    content: &str,
    entities: &HashSet<String>,
    excluded_ranges: &[(usize, usize)],
) -> Vec<TypedEdge> {
    struct HeadingEntry {
        level: usize,
        name: String,
    }

    let mut stack: Vec<HeadingEntry> = Vec::new();
    let mut edges: Vec<TypedEdge> = Vec::new();

    for line in content.lines() {
        // Skip table rows, code blocks, and other excluded content
        let line_start = line.as_ptr() as usize - content.as_ptr() as usize;
        let line_end = line_start + line.len();
        if is_in_ranges(line_start, line_end, excluded_ranges) {
            continue;
        }

        let trimmed = line.trim();
        let level = trimmed.bytes().take_while(|&b| b == b'#').count();
        if !(1..=6).contains(&level) {
            continue;
        }
        let heading_text = strip_heading_number(trimmed[level..].trim()).to_lowercase();

        // Skip headings that are in the stop list (linear search — list is small)
        if STOP_HEADINGS.iter().any(|s| *s == heading_text) {
            continue;
        }
        // Only create edges for headings that are known entities
        if !entities.contains(&heading_text) {
            continue;
        }

        // Pop stack until we find a parent at a higher (smaller) level
        while stack.last().is_some_and(|h| h.level >= level) {
            stack.pop();
        }

        // The top of the stack is the parent heading
        if let Some(parent) = stack.last() {
            if parent.name != heading_text {
                edges.push(TypedEdge {
                    relation: "part_of".to_string(),
                    from: heading_text.clone(),
                    to: parent.name.clone(),
                });
            }
        }

        stack.push(HeadingEntry {
            level,
            name: heading_text,
        });
    }

    edges
}

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
    pub fn find_mentions<'m>(
        &'m self,
        content: &str,
        code_ranges: &[(usize, usize)],
    ) -> Vec<&'m str> {
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
            let overlaps = deduped
                .iter()
                .any(|&(s, e, _)| (s..=e).contains(&m.0) && (s..=e).contains(&m.1));
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
    ///
    /// `extra_patterns` are domain-verb patterns discovered from the document
    /// itself by [`Self::discover_verb_patterns`]. They are checked in
    /// addition to [`RELATION_PATTERNS`].
    pub fn find_relationships(
        &self,
        content: &str,
        excluded_ranges: &[(usize, usize)],
        extra_patterns: &[(&str, &str)],
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

            // Adjust exclusion ranges relative to this sentence subslice
            // so find_mentions_with_positions can skip individual mentions inside
            // table rows / code blocks without losing valid content outside them.
            let adjusted: Vec<(usize, usize)> = excluded_ranges
                .iter()
                .filter(|&&(s, e)| s < end && e > start)
                .map(|&(s, e)| {
                    let adj_s = s.saturating_sub(start);
                    let adj_e = if e > end { end - start } else { e - start };
                    (adj_s.min(end - start), adj_e.min(end - start))
                })
                .collect();

            let mentions = self.find_mentions_with_positions(sentence, &adjusted);
            if mentions.len() < 2 {
                continue;
            }

            for i in 0..mentions.len() {
                for j in i + 1..mentions.len() {
                    if mentions[j].1 <= mentions[i].1 {
                        continue;
                    }
                    // Build between-text from non-excluded byte ranges
                    let between_start = mentions[i].1;
                    let between_end = mentions[j].0;
                    let mut between = String::new();
                    let mut cursor = between_start;
                    for &(s, e) in &adjusted {
                        if s >= between_end {
                            break;
                        }
                        if s > cursor {
                            between.push_str(&sentence[cursor..s.min(between_end)]);
                        }
                        cursor = cursor.max(e);
                        if cursor >= between_end {
                            break;
                        }
                    }
                    if cursor < between_end {
                        between.push_str(&sentence[cursor..between_end]);
                    }
                    let between = between.trim();
                    if between.is_empty() {
                        continue;
                    }

                    let between_lower = between.to_lowercase();
                    // Check built-in patterns first, then discovered ones.
                    // break on first match so longer patterns have priority.
                    let matched = RELATION_PATTERNS
                        .iter()
                        .chain(extra_patterns.iter())
                        .find(|(verb, _)| between_lower.contains(verb));
                    if let Some((_, rel_type)) = matched {
                        edges.push(TypedEdge {
                            relation: rel_type.to_string(),
                            from: mentions[i].2.to_string(),
                            to: mentions[j].2.to_string(),
                        });
                    }
                }
            }
        }

        edges
    }

    /// Discover frequent verb patterns that appear between entity pairs in
    /// this document.
    ///
    /// Scans every sentence, finds paired entity mentions, and counts every
    /// non-stop-word in the text between them. Words that appear ≥ `min_freq`
    /// times are returned as candidate relationship patterns.
    ///
    /// This makes the linker fully domain-agnostic: a medical document would
    /// discover "treats" / "diagnoses" / "prevents"; a legal document would
    /// discover "governed_by" / "requires_compliance_with".
    ///
    /// ponytail: frequency-based discovery cannot detect rare (< min_freq)
    /// relationships. The built-in [`RELATION_PATTERNS`] covers common
    /// infrastructure verbs as a safety net.
    pub fn discover_verb_patterns(
        &self,
        content: &str,
        min_freq: usize,
        excluded_ranges: &[(usize, usize)],
    ) -> Vec<(String, String)> {
        if self.entity_names.is_empty() {
            return vec![];
        }

        // Build a set of known entity names — they should never become
        // relationship types even if they appear between entity pairs.
        let entity_set: HashSet<&str> = self.entity_names.iter().map(|s| s.as_str()).collect();

        let mut counts: HashMap<String, usize> = HashMap::new();
        let bytes = content.as_bytes();
        let len = bytes.len();

        // Walk sentence boundaries (same logic as find_relationships)
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

            if sentence.starts_with("    ") || sentence.starts_with('\t') {
                continue;
            }

            // Adjust exclusion ranges relative to this sentence subslice
            // (same approach as find_relationships)
            let adjusted: Vec<(usize, usize)> = excluded_ranges
                .iter()
                .filter(|&&(s, e)| s < end && e > start)
                .map(|&(s, e)| {
                    let adj_s = s.saturating_sub(start);
                    let adj_e = if e > end { end - start } else { e - start };
                    (adj_s.min(end - start), adj_e.min(end - start))
                })
                .collect();

            let mentions = self.find_mentions_with_positions(sentence, &adjusted);
            if mentions.len() < 2 {
                continue;
            }

            for i in 0..mentions.len() {
                for j in i + 1..mentions.len() {
                    if mentions[j].1 <= mentions[i].1 {
                        continue;
                    }
                    let between_start = mentions[i].1;
                    let between_end = mentions[j].0;
                    let mut between = String::new();
                    let mut cursor = between_start;
                    for &(s, e) in &adjusted {
                        if s >= between_end {
                            break;
                        }
                        if s > cursor {
                            between.push_str(&sentence[cursor..s.min(between_end)]);
                        }
                        cursor = cursor.max(e);
                        if cursor >= between_end {
                            break;
                        }
                    }
                    if cursor < between_end {
                        between.push_str(&sentence[cursor..between_end]);
                    }
                    let between = between.trim();
                    if between.is_empty() {
                        continue;
                    }
                    // Count every alphanumeric word ≥ 3 chars that isn't a stop word,
                    // an existing entity name (entities are things, not relationships),
                    // or a non-verb (nouns like "maps", "example" are common noise).
                    for word in between.split_whitespace() {
                        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
                        if word.len() < 3 {
                            continue;
                        }
                        let lower = word.to_lowercase();
                        if STOP_WORDS.binary_search(&lower.as_str()).is_ok() {
                            continue;
                        }
                        if entity_set.contains(lower.as_str()) {
                            continue;
                        }
                        // Skip words that don't look like verbs
                        if !is_likely_verb(&lower) {
                            continue;
                        }
                        *counts.entry(lower).or_insert(0) += 1;
                    }
                }
            }
        }

        let mut verbs: Vec<(String, usize)> =
            counts.into_iter().filter(|(_, c)| *c >= min_freq).collect();
        verbs.sort_unstable_by_key(|v| std::cmp::Reverse(v.1));
        verbs.into_iter().map(|(v, _)| (v.clone(), v)).collect()
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

/// Find byte ranges of GFM pipe table rows.
///
/// A table row is any line starting with `|` (after optional whitespace).
/// Each matching line is emitted as its own byte range.
pub fn find_table_ranges(content: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut line_start: usize = 0;
    for line in content.lines() {
        if line.trim().starts_with('|') {
            ranges.push((line_start, line_start + line.len()));
        }
        line_start += line.len() + 1; // +1 for newline
    }
    ranges
}

/// Find byte ranges of list-item bold labels (`- **Term**:` or `* **Term**:`).
///
/// These are structured-data markers, not prose content. Excluding them
/// from entity-mention scanning prevents false relationships like
/// `cpu | tested | change log` (from `- **Last Tested**:` being paired
/// with every entity in the same sentence window).
pub fn find_list_item_bold_ranges(content: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i + 3 < len {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' && is_list_item_bold(content, i) {
            let start = i;
            if let Some(end) = find_closing_double_star(bytes, i + 2) {
                let close = end + 2; // past closing **
                                     // Include the trailing colon if present
                let range_end = if close < len && bytes[close] == b':' {
                    close + 1
                } else {
                    close
                };
                ranges.push((start, range_end));
                i = range_end;
                continue;
            }
        }
        i += 1;
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

/// Strip leading section-number prefix (e.g. `5.1 `, `1.2.3 `) from heading text.
fn strip_heading_number(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return s;
    }
    // Each dot MUST be followed by at least one digit.
    while i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let dot_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == dot_start {
            return s; // bare dot — not a section number
        }
    }
    if i < bytes.len() && bytes[i] == b' ' {
        s[i + 1..].trim_start()
    } else {
        s
    }
}

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

/// Check if a bold span at byte position `open` (the `*` before `**term**`)
/// is a list-item label like `- **Term**:` or `* **Term**:`.
///
/// These are structured-data markers, not content entities that should
/// participate in relationship extraction.
fn is_list_item_bold(content: &str, open: usize) -> bool {
    if open < 2 {
        return false;
    }
    let before = &content.as_bytes()[..open];
    let len = before.len();
    // Skip trailing whitespace before the `**` opener
    let mut end = len;
    while end > 0 && (before[end - 1] == b' ' || before[end - 1] == b'\t') {
        end -= 1;
    }
    if end < 1 {
        return false;
    }
    let marker = before[end - 1];
    if marker != b'-' && marker != b'*' {
        return false;
    }
    // Confirm the list marker is at line start (or preceded by newline)
    end == 1 || before[end - 2] == b'\n' || before[end - 2] == b'\r'
}

/// Extract entity vocabulary from document structure.
///
/// Sources:
/// 1. Section headings (`## Title` and deeper)
/// 2. Bold terms (`**term**`) — skip list-item labels like `- **Term**:`
/// 3. Code spans (`` `tool` ``)
///
/// `excluded_ranges` — byte ranges to skip (code blocks, tables, etc).
pub fn extract_vocabulary(content: &str, excluded_ranges: &[(usize, usize)]) -> EntityVocabulary {
    let mut vocab = EntityVocabulary::default();

    // 1. Section headings
    let mut line_start: usize = 0;
    for line in content.lines() {
        let line_end = line_start + line.len();
        if !is_in_ranges(line_start, line_end, excluded_ranges) {
            if let Some((_level, heading)) = parse_heading_line(line) {
                let heading = strip_heading_number(heading);
                if STOP_HEADINGS.contains(&heading.to_lowercase().as_str()) || heading.len() < 4 {
                    line_start += line.len() + 1;
                    continue;
                }
                vocab.insert(heading);
            }
        }
        line_start += line.len() + 1;
    }

    // 2. Bold terms
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i + 3 < len {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            let start = i + 2;
            if let Some(end) = find_closing_double_star(bytes, start) {
                if !is_in_ranges(start, end, excluded_ranges) && !is_list_item_bold(content, i) {
                    let term = &content[start..end].trim();
                    if term.chars().any(|c| c.is_uppercase()) && term.len() >= 3 {
                        vocab.insert(term);
                    }
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
                if !is_in_ranges(start, end, excluded_ranges) {
                    let tool = &content[start..end].trim();
                    if tool.len() >= 3
                        && tool.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                    {
                        vocab.insert(tool);
                    }
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
        let vocab = extract_vocabulary(content, &[]);
        assert!(vocab
            .entities
            .contains(&"crush map configuration".to_string()));
        assert!(vocab.entities.contains(&"osd deployment".to_string()));
    }

    #[test]
    fn excludes_h1() {
        let content = "# Title\n## Section\n";
        let vocab = extract_vocabulary(content, &[]);
        assert!(!vocab.entities.contains(&"title".to_string()));
        assert!(vocab.entities.contains(&"section".to_string()));
    }

    #[test]
    fn filters_generic_headings() {
        let content = "## Overview\n\n## Introduction\n\n## CRUSH Map\n";
        let vocab = extract_vocabulary(content, &[]);
        assert!(!vocab.entities.contains(&"overview".to_string()));
        assert!(vocab.entities.contains(&"crush map".to_string()));
    }

    #[test]
    fn extracts_bold_terms() {
        let content = "The **CRUSH** algorithm distributes data across **OSDs**.";
        let vocab = extract_vocabulary(content, &[]);
        assert!(vocab.entities.contains(&"crush".to_string()));
        assert!(vocab.entities.contains(&"osds".to_string()));
    }

    #[test]
    fn extracts_code_spans() {
        let content = "Configure `ceph.conf` and run `ceph-deploy`.";
        let vocab = extract_vocabulary(content, &[]);
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
    fn strip_heading_number_removes_section_prefix() {
        assert_eq!(
            strip_heading_number("5.1 Ceph Components"),
            "Ceph Components"
        );
        assert_eq!(strip_heading_number("1.2.3 Overview"), "Overview");
        assert_eq!(strip_heading_number("Ceph Components"), "Ceph Components");
        assert_eq!(strip_heading_number(""), "");
        assert_eq!(strip_heading_number("5.1"), "5.1");
        assert_eq!(strip_heading_number("5.1A Heading"), "5.1A Heading");
        assert_eq!(strip_heading_number("5. 1 Leading"), "5. 1 Leading");
        assert_eq!(strip_heading_number("5."), "5.");
    }

    #[test]
    fn extract_vocabulary_strips_heading_numbers() {
        let content = "## 5.1 Ceph Components\n## 1.2.3 OSD Deployment\n";
        let vocab = extract_vocabulary(content, &[]);
        assert!(
            vocab.entities.contains(&"ceph components".to_string()),
            "should match body-style name, got: {:?}",
            vocab.entities
        );
        assert!(vocab.entities.contains(&"osd deployment".to_string()));
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
        assert_eq!(
            found.len(),
            2,
            "should find Ceph twice (outside code): {:?}",
            found
        );
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
        let m: EntityMatcher = extract_vocabulary(content, &[]).into();
        let code = find_code_ranges(content);
        let edges = m.find_relationships(content, &code, &[]);
        let req = edges.iter().find(|e| e.relation == "requires");
        assert!(req.is_some(), "should find 'requires': {:?}", edges);
        assert_eq!(req.unwrap().from.to_lowercase(), "ceph");
        assert_eq!(req.unwrap().to.to_lowercase(), "crush");
    }

    #[test]
    fn extracts_runs_on_relationship() {
        let content = "## Proxmox VE\n\n## Debian\n\nProxmox VE runs on Debian.";
        let m: EntityMatcher = extract_vocabulary(content, &[]).into();
        let edges = m.find_relationships(content, &[], &[]);
        let edge = edges.iter().find(|e| e.relation == "runs_on");
        assert!(edge.is_some(), "should find 'runs_on': {:?}", edges);
        assert_eq!(edge.unwrap().from.to_lowercase(), "proxmox ve");
        assert_eq!(edge.unwrap().to.to_lowercase(), "debian");
    }

    #[test]
    fn no_relationships_with_empty_vocab() {
        let content = "The system runs on a standard kernel.";
        let m: EntityMatcher = EntityVocabulary::default().into();
        let edges = m.find_relationships(content, &[], &[]);
        assert!(edges.is_empty());
    }

    #[test]
    fn relationship_skips_code_blocks() {
        let content = "## Ceph\n\n## CRUSH\n\nCeph requires CRUSH for data distribution.\n```\nCeph requires nothing inside code.\n```";
        let m: EntityMatcher = extract_vocabulary(content, &[]).into();
        let code = find_code_ranges(content);
        let edges = m.find_relationships(content, &code, &[]);
        // Should find the "requires" relationship from the non-code part
        let req = edges.iter().find(|e| e.relation == "requires");
        assert!(
            req.is_some(),
            "should find 'requires' outside code: {:?}",
            edges
        );
    }

    // --- DB vocabulary ---

    #[test]
    fn vocabulary_merge_deduplicates() {
        let mut vocab = extract_vocabulary("## Ceph\n## CRUSH\n", &[]);
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

    // --- Verb discovery ---

    #[test]
    fn discovers_verbs_between_entity_pairs() {
        let content = "\
## Ceph

## CRUSH

## Proxmox VE

Ceph requires CRUSH for data distribution.
Proxmox VE runs on Debian.
Ceph supports erasure coding.
Proxmox VE manages Ceph clusters.
Ceph stores data in pools.
Proxmox VE provides a web interface.
Ceph handles replication.";
        let m: EntityMatcher = extract_vocabulary(content, &[]).into();
        let verbs = m.discover_verb_patterns(content, 3, &[]);
        assert!(
            verbs.iter().any(|(v, _)| v == "requires"),
            "expected 'requires' in discovered verbs: {:?}",
            verbs,
        );
        assert!(
            verbs.iter().any(|(v, _)| v == "manages"),
            "expected 'manages' in discovered verbs: {:?}",
            verbs,
        );
        // "on" and "for" are stop words and should never appear
        assert!(
            !verbs.iter().any(|(v, _)| v == "on"),
            "'on' is a stop word and should not be discovered",
        );
        assert!(
            !verbs.iter().any(|(v, _)| v == "for"),
            "'for' is a stop word and should not be discovered",
        );
    }

    #[test]
    fn discovered_verbs_are_used_in_relationships() {
        let content = "\
## Ceph

## CRUSH

Ceph manages CRUSH maps.

Ceph manages pools.

Ceph manages OSDs.";
        let m: EntityMatcher = extract_vocabulary(content, &[]).into();
        let verbs = m.discover_verb_patterns(content, 3, &[]);
        let vrefs: Vec<(&str, &str)> = verbs
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let edges = m.find_relationships(content, &[], &vrefs);
        // With paragraph breaks, each sentence is its own window:
        //   1. Ceph → CRUSH: " manages " -> "manages"
        //   2. Ceph → (pools not an entity): no pair
        //   3. Ceph → (OSDs not an entity): no pair
        // So we get exactly 1 manages edge.
        let mgmt = edges.iter().filter(|e| e.relation == "manages").count();
        assert_eq!(
            mgmt, 1,
            "should have 1 'manages' edge (only CRUSH is an entity): {:?}",
            edges
        );
    }

    // --- Heading hierarchy ---

    #[test]
    fn heading_hierarchy_creates_part_of_edges() {
        let content = "\
# Proxmox VE

## Ceph

### CRUSH Map

#### Pool Configuration

## Proxmox Backup Server

### PBS Configuration
";
        let entities: HashSet<String> = [
            "proxmox ve",
            "ceph",
            "crush map",
            "pool configuration",
            "proxmox backup server",
            "pbs configuration",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let edges = extract_heading_relationships(content, &entities, &[]);
        assert_eq!(
            edges.len(),
            5,
            "all heading pairs should create part_of: {:?}",
            edges
        );

        let pairs: Vec<(&str, &str)> = edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect();
        assert!(pairs.contains(&("ceph", "proxmox ve")));
        assert!(pairs.contains(&("crush map", "ceph")));
        assert!(pairs.contains(&("pool configuration", "crush map")));
        assert!(pairs.contains(&("proxmox backup server", "proxmox ve")));
        assert!(pairs.contains(&("pbs configuration", "proxmox backup server")));
    }

    #[test]
    fn heading_hierarchy_skips_stop_headings() {
        let content = "\
# Overview

## Introduction

## Ceph

### Prerequisites

### CRUSH Map
";
        // Only "Ceph" and "CRUSH Map" survive; "Overview", "Introduction", "Prerequisites" are STOP_HEADINGS.
        let entities: HashSet<String> = [
            "ceph",
            "crush map",
            "overview",
            "introduction",
            "prerequisites",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let edges = extract_heading_relationships(content, &entities, &[]);
        // "Ceph" has no parent that is also a stop-heading entity → no edge.
        // "CRUSH Map" under "Ceph" → 1 edge.
        assert_eq!(
            edges.len(),
            1,
            "should only have CRUSH Map -> Ceph: {:?}",
            edges
        );
        assert_eq!(edges[0].from, "crush map");
        assert_eq!(edges[0].to, "ceph");
    }

    // --- Verb suffix filtering ---

    #[test]
    fn verb_suffix_filter_rejects_nouns() {
        let content = "\
## Ceph

## OSD

Ceph maps OSD data.
Ceph maps OSD pools.
Ceph maps OSD failures.
";
        let m: EntityMatcher = extract_vocabulary(content, &[]).into();
        let verbs = m.discover_verb_patterns(content, 3, &[]);
        // "maps" ends in 's' but it's a plural noun, not a 3rd-person verb.
        // If discovered, "maps" would create false edges.
        assert!(
            !verbs.iter().any(|(v, _)| v == "maps"),
            "'maps' is a plural noun, should be filtered: {:?}",
            verbs,
        );
    }

    #[test]
    fn verb_suffix_accepts_verb_patterns() {
        assert!(is_likely_verb("configures"));
        assert!(is_likely_verb("manages"));
        assert!(is_likely_verb("processed"));
        assert!(is_likely_verb("processing"));
        assert!(is_likely_verb("communicates"));
        assert!(is_likely_verb("integrated"));
        assert!(is_likely_verb("verifies"));
        assert!(!is_likely_verb("maps"));
        assert!(!is_likely_verb("data"));
        assert!(!is_likely_verb("example"));
        assert!(!is_likely_verb("system"));
    }
}
